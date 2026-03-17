#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{env, ipc, process, sys};
use anyos_std::shell;
use anyos_std::format;

fn main() -> u32 {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count == 0 {
        sys::con_write("usage: sudo <command> [args...]\n");
        return 1;
    }

    // Prompt for the root password.
    sys::con_write("[sudo] password for root: ");
    let mut pw_buf = [0u8; 128];
    let pw_len = sys::con_read_password(&mut pw_buf);
    sys::con_write("\n");
    let password = core::str::from_utf8(&pw_buf[..pw_len]).unwrap_or("");

    if !process::authenticate("root", password) {
        sys::con_write("sudo: authentication failed\n");
        return 1;
    }

    // Elevate this process to root for the duration of the command.
    let original_uid = process::getuid();
    process::set_identity(0);

    // Rebuild the full command string from the positional arguments.
    let cmd = args.positional[0];
    let mut full_args = anyos_std::String::from(cmd);
    for i in 1..args.pos_count {
        full_args.push(' ');
        full_args.push_str(args.positional[i]);
    }

    // Resolve the executable path via $PATH.
    let mut cwd_buf = [0u8; 256];
    let cwd_len = env::get("PWD", &mut cwd_buf);
    let cwd = if cwd_len != u32::MAX && cwd_len > 0 {
        core::str::from_utf8(&cwd_buf[..cwd_len as usize]).unwrap_or("/")
    } else {
        "/"
    };
    let prog_path = shell::resolve_cmd_path(cmd, cwd);

    // Spawn with a pipe so stdout flows back to the calling console.
    let pipe_name = format!("sudo:out:{}", original_uid);
    let out_pipe = ipc::pipe_create(&pipe_name);
    if out_pipe == 0 || out_pipe == u32::MAX {
        sys::con_write("sudo: failed to create output pipe\n");
        process::set_identity(original_uid);
        return 1;
    }

    let in_pipe_name = format!("sudo:in:{}", original_uid);
    let in_pipe = ipc::pipe_create(&in_pipe_name);

    let tid = process::spawn_piped_full(&prog_path, &full_args, out_pipe, in_pipe);
    if tid == u32::MAX {
        ipc::pipe_close(out_pipe);
        ipc::pipe_close(in_pipe);
        let msg = format!("sudo: {}: command not found\n", cmd);
        sys::con_write(&msg);
        process::set_identity(original_uid);
        return 1;
    }

    // Pump output to the console until the child exits.
    let mut buf = [0u8; 512];
    'pump: loop {
        loop {
            let n = ipc::pipe_read(out_pipe, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
                sys::con_write(s);
            }
        }

        if sys::con_poll_key() == 0x03 {
            process::kill(tid);
            sys::con_write("\n^C\n");
            drain(out_pipe, &mut buf);
            break 'pump;
        }

        if process::try_waitpid(tid) != process::STILL_RUNNING {
            drain(out_pipe, &mut buf);
            break;
        }

        process::sleep(10);
    }

    ipc::pipe_close(out_pipe);
    ipc::pipe_close(in_pipe);
    sys::con_set_mode(0);

    // Restore the original identity — sudo does not permanently elevate.
    process::set_identity(original_uid);

    0
}

fn drain(pipe: u32, buf: &mut [u8]) {
    loop {
        let n = ipc::pipe_read(pipe, buf);
        if n == 0 || n == u32::MAX { break; }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            sys::con_write(s);
        }
    }
}
