//! Session lifecycle helpers.

use anyos_std::println;
use anyos_std::process;
use anyos_std::Vec;

use crate::render::{acquire_lock, desktop_ref, release_lock, signal_render};

pub(crate) fn perform_logout(
    login_tid: &mut u32,
    login_pending: &mut bool,
    dock_spawned: &mut bool,
    service_tids: &mut Vec<u32>,
) {
    println!("compositor: logout requested — terminating user processes...");

    let mut tids_to_kill: Vec<u32>;
    {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        tids_to_kill = Vec::with_capacity(desktop.windows.len() + desktop.app_subs.len());
        for win in &desktop.windows {
            if win.owner_tid != 0 && !tids_to_kill.contains(&win.owner_tid) {
                tids_to_kill.push(win.owner_tid);
            }
        }
        for &(tid, _) in &desktop.app_subs {
            if tid != 0 && !tids_to_kill.contains(&tid) {
                tids_to_kill.push(tid);
            }
        }
        release_lock();
    }

    for &tid in service_tids.iter() {
        if !tids_to_kill.contains(&tid) {
            tids_to_kill.push(tid);
        }
    }
    service_tids.clear();

    for &tid in &tids_to_kill {
        process::kill(tid);
    }
    process::sleep(200);

    {
        acquire_lock();
        let desktop = unsafe { desktop_ref() };
        let remaining: Vec<u32> = desktop.windows.iter().map(|w| w.id).collect();
        for id in remaining {
            desktop.destroy_window(id);
        }

        desktop.app_subs.clear();
        desktop.menu_bar = crate::menu::MenuBar::new();
        desktop.focused_window = None;
        desktop.tray_ipc_events.clear();
        desktop.clipboard_data.clear();
        desktop.set_menubar_visible(false);
        desktop.reload_wallpaper_and_icons();
        desktop.compositor.damage_all();
        release_lock();
    }
    signal_render();

    let new_tid = process::spawn("/System/login", "");
    if new_tid != u32::MAX {
        *login_tid = new_tid;
        *login_pending = true;
        *dock_spawned = false;
        println!("compositor: logged out, login re-spawned (TID={})", new_tid);
    } else {
        println!("compositor: FATAL — cannot spawn login after logout");
    }
}
