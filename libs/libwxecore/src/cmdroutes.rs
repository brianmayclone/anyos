//! cmd.exe import map and route table.
//!
//! `CMD_IMPORTS` is the exact per-DLL import list of the bundled Windows 11
//! cmd.exe (extracted from its PE import + delay-import tables). `IMPL` maps the
//! functions wxe actually implements to a `wxedll` route string. Everything not
//! in `IMPL` is left unrouted: the kernel PE loader points it at a logged
//! return-0 stub, so the still-missing surface shows up in `anyos.log` instead
//! of aborting the load.
//!
//! Routes are emitted directly on the importing DLL (api-set or ntdll), so no
//! forwarder chain or separate kernelbase/ucrtbase implementation file is
//! required for cmd.exe.

use alloc::string::String;

/// name -> wxedll route. Only functions with non-trivial behavior (or a
/// non-zero return / valid pointer) need an entry; everything else falls back
/// to the loader's logged 0-stub.
const IMPL: &[(&str, &str)] = &[
    // ---- ntdll (imported directly by cmd) ----
    ("NtClose", "nt:0x0003"),
    ("NtOpenFile", "nt:0x000f"),
    ("NtQueryInformationProcess", "nt:0x000c"),
    // ---- process / exit ----
    ("TerminateProcess", "nt:0x0001"),
    ("GetCommandLineW", "win32:process:cmdline-w"),
    ("GetCommandLineA", "win32:process:cmdline-a"),
    ("GetCurrentProcess", "win32:pseudo:process"),
    ("GetCurrentThread", "win32:pseudo:thread"),
    ("GetCurrentProcessId", "win32:id:process"),
    ("GetCurrentThreadId", "win32:id:thread"),
    // ---- environment ----
    ("GetEnvironmentStringsW", "win32:env:block-w"),
    ("FreeEnvironmentStringsW", "win32:env:free"),
    ("GetEnvironmentVariableW", "win32:env:get-w"),
    ("SetEnvironmentVariableW", "win32:env:set-w"),
    ("ExpandEnvironmentStringsW", "win32:env:expand-w"),
    ("SetEnvironmentStringsW", "stub:one"),
    // ---- module / loader ----
    ("GetModuleHandleW", "win32:module:handle-w"),
    ("GetProcAddress", "win32:module:proc"),
    ("GetModuleFileNameW", "win32:module:filename-w"),
    ("LoadLibraryExW", "win32:module:load-ex-w"),
    // ---- console ----
    ("GetStdHandle", "win32:console:std-handle"),
    ("WriteConsoleW", "win32:console:write-con-w"),
    ("ReadConsoleW", "win32:console:read-con-w"),
    ("WriteConsoleA", "win32:console:write-con-a"),
    ("GetConsoleMode", "win32:console:get-mode"),
    ("SetConsoleMode", "win32:console:set-mode"),
    ("GetConsoleOutputCP", "win32:console:output-cp"),
    ("GetConsoleCP", "win32:console:output-cp"),
    ("GetConsoleScreenBufferInfo", "win32:console:screen-info"),
    ("SetConsoleCtrlHandler", "win32:console:ctrl-handler"),
    ("GetConsoleWindow", "win32:console:window"),
    ("SetConsoleTextAttribute", "stub:one"),
    ("SetConsoleTitleW", "stub:one"),
    ("SetConsoleCursorPosition", "stub:one"),
    ("FillConsoleOutputAttribute", "stub:one"),
    ("FillConsoleOutputCharacterW", "stub:one"),
    ("ScrollConsoleScreenBufferW", "stub:one"),
    ("FlushConsoleInputBuffer", "stub:one"),
    // ---- files ----
    ("CreateFileW", "win32:file:create-w"),
    ("ReadFile", "win32:file:read"),
    ("WriteFile", "win32:file:write"),
    ("CloseHandle", "win32:handle:close"),
    ("GetFileType", "win32:file:type"),
    ("GetFileAttributesW", "win32:file:attributes-w"),
    ("SetFilePointerEx", "win32:file:pointer-ex"),
    // ---- memory / heap ----
    ("VirtualAlloc", "win32:memory:alloc"),
    ("VirtualFree", "win32:memory:free"),
    ("HeapAlloc", "win32:heap:alloc"),
    ("HeapFree", "win32:heap:free"),
    ("HeapReAlloc", "win32:heap:realloc"),
    ("HeapSize", "win32:heap:size"),
    ("GetProcessHeap", "win32:heap:process"),
    ("HeapSetInformation", "stub:one"),
    // ---- time ----
    ("Sleep", "win32:time:sleep"),
    ("GetSystemTimeAsFileTime", "win32:time:filetime"),
    ("QueryPerformanceCounter", "win32:time:qpc"),
    // ---- startup / directory / locale ----
    ("GetStartupInfoW", "win32:startup-info-w"),
    ("GetCurrentDirectoryW", "win32:dir:get-w"),
    ("SetCurrentDirectoryW", "win32:dir:set-w"),
    ("GetACP", "win32:acp"),
    ("GetOEMCP", "win32:acp"),
    ("GetCPInfo", "stub:one"),
    // ---- sync primitives that must report success (single-threaded no-ops) ----
    ("InitializeCriticalSectionEx", "stub:one"),
    ("CreateSemaphoreExW", "stub:one"),
    ("CreateMutexExW", "stub:one"),
    // ---- UCRT (api-ms-win-crt-*, the `_o_` thunks plus plain mem*) ----
    ("memcpy", "ucrt:memcpy"),
    ("memmove", "ucrt:memmove"),
    ("memset", "ucrt:memset"),
    ("memcmp", "ucrt:memcmp"),
    ("_initterm", "ucrt:initterm"),
    ("_initterm_e", "ucrt:initterm-e"),
    ("_o_malloc", "ucrt:malloc"),
    ("_o_free", "ucrt:free"),
    ("_o_realloc", "ucrt:realloc"),
    ("_o_calloc", "ucrt:calloc"),
    ("_o__get_initial_narrow_environment", "ucrt:narrow-env-get"),
    ("_o___p___argc", "ucrt:argc-ptr"),
    ("_o___p___argv", "ucrt:argv-ptr"),
    ("_o__errno", "ucrt:errno-ptr"),
    ("_o___p__commode", "ucrt:errno-ptr"),
    ("_o___acrt_iob_func", "ucrt:iob"),
    ("_o___stdio_common_vswprintf", "ucrt:vswprintf"),
    ("_o___stdio_common_vswprintf_s", "ucrt:vswprintf"),
    ("_o__get_osfhandle", "win32:osf-handle"),
    ("_o_exit", "ucrt:exit"),
    ("_o__exit", "ucrt:exit"),
    ("_o__purecall", "ucrt:exit"),
    ("_o_terminate", "ucrt:exit"),
];

/// Build the `dll!name=route` manifest for every cmd.exe import we implement.
pub fn build_cmd_routes() -> String {
    let mut out = String::new();
    for (dll, names) in CMD_IMPORTS {
        for name in *names {
            if let Some(route) = impl_route(name) {
                out.push_str(dll);
                out.push('!');
                out.push_str(name);
                out.push('=');
                out.push_str(route);
                out.push('\n');
            }
        }
    }
    out
}

/// Every DLL cmd.exe imports from (so the loader finds a real file per import).
pub fn cmd_dlls() -> impl Iterator<Item = &'static str> {
    CMD_IMPORTS.iter().map(|(dll, _)| *dll)
}

fn impl_route(name: &str) -> Option<&'static str> {
    IMPL.iter().find(|(n, _)| *n == name).map(|(_, r)| *r)
}

/// cmd.exe's exact import surface (DLL -> imported function names).
const CMD_IMPORTS: &[(&str, &[&str])] = &[
    ("api-ms-win-crt-string-l1-1-0.dll", &[
        "wcscmp",
        "wcsncmp",
        "memset",
        "wcsspn",
    ]),
    ("api-ms-win-crt-time-l1-1-0.dll", &[
        "_time32",
    ]),
    ("api-ms-win-crt-runtime-l1-1-0.dll", &[
        "_initterm",
        "_initterm_e",
        "_register_thread_local_exe_atexit_callback",
        "_c_exit",
    ]),
    ("api-ms-win-crt-private-l1-1-0.dll", &[
        "_o__get_initial_narrow_environment",
        "_o__get_osfhandle",
        "_o__getch",
        "_o__initialize_narrow_environment",
        "_o__initialize_onexit_table",
        "_o__invalid_parameter_noinfo",
        "_o__open_osfhandle",
        "_o__pclose",
        "_o__pipe",
        "_o__purecall",
        "_o__register_onexit_function",
        "_o__seh_filter_exe",
        "_o__set_app_type",
        "_o__set_fmode",
        "_o__set_new_mode",
        "_o__setmode",
        "_o__tell",
        "_o__configure_narrow_argv",
        "_o__ultoa",
        "_o__ultoa_s",
        "__intrinsic_setjmp",
        "_o__wcsicmp",
        "_o__wcslwr",
        "_o__wcsnicmp",
        "_o__wcsupr",
        "_o__wpopen",
        "_o__wtol",
        "_o_calloc",
        "_o_exit",
        "_o_feof",
        "_o_ferror",
        "_o_fflush",
        "_o_fgets",
        "_o_free",
        "_o_iswalpha",
        "_o_iswdigit",
        "_o_iswspace",
        "_o_iswxdigit",
        "_o_malloc",
        "_o_qsort",
        "_o_rand",
        "_o_realloc",
        "_o_setlocale",
        "_o_srand",
        "_o_terminate",
        "_o_towlower",
        "_o_towupper",
        "_o_wcstol",
        "_o_wcstoul",
        "__CxxFrameHandler3",
        "__current_exception",
        "__current_exception_context",
        "_o__configthreadlocale",
        "_CxxThrowException",
        "_o__close",
        "_o__cexit",
        "_o__callnewh",
        "_o___stdio_common_vswscanf",
        "_o___stdio_common_vswprintf_s",
        "_o___stdio_common_vswprintf",
        "_o__exit",
        "_o__errno",
        "_o___stdio_common_vfprintf",
        "_o__dup2",
        "_o__dup",
        "_o___std_exception_destroy",
        "_o___std_exception_copy",
        "_o___p__commode",
        "_o___p___argv",
        "_o___p___argc",
        "_o___acrt_iob_func",
        "wcsstr",
        "wcsrchr",
        "wcschr",
        "longjmp",
        "_o__crt_atexit",
        "__C_specific_handler",
        "_local_unwind",
        "memcmp",
        "memcpy",
        "memmove",
    ]),
    ("ntdll.dll", &[
        "RtlLookupFunctionEntry",
        "RtlVirtualUnwind",
        "NtOpenProcessToken",
        "RtlReleaseRelativeName",
        "RtlCreateUnicodeStringFromAsciiz",
        "NtClose",
        "NtOpenThreadToken",
        "NtCancelSynchronousIoFile",
        "RtlNtStatusToDosError",
        "NtQueryInformationProcess",
        "RtlFreeUnicodeString",
        "NtSetInformationProcess",
        "NtQueryVolumeInformationFile",
        "RtlFindLeastSignificantBit",
        "RtlDosPathNameToNtPathName_U",
        "NtSetInformationFile",
        "RtlDosPathNameToRelativeNtPathName_U_WithStatus",
        "NtQueryInformationToken",
        "NtOpenFile",
        "NtFsControlFile",
        "RtlFreeHeap",
        "RtlCaptureContext",
    ]),
    ("api-ms-win-core-libraryloader-l1-2-0.dll", &[
        "LoadLibraryExW",
        "GetModuleFileNameA",
        "GetModuleFileNameW",
        "GetModuleHandleW",
        "GetProcAddress",
        "GetModuleHandleExW",
    ]),
    ("api-ms-win-core-synch-l1-1-0.dll", &[
        "ReleaseSemaphore",
        "CreateSemaphoreExW",
        "InitializeCriticalSection",
        "LeaveCriticalSection",
        "DeleteCriticalSection",
        "AcquireSRWLockShared",
        "CreateMutexExW",
        "InitializeCriticalSectionEx",
        "ReleaseSRWLockShared",
        "EnterCriticalSection",
        "OpenSemaphoreW",
        "WaitForSingleObject",
        "AcquireSRWLockExclusive",
        "TryAcquireSRWLockExclusive",
        "ReleaseSRWLockExclusive",
        "ReleaseMutex",
        "WaitForSingleObjectEx",
    ]),
    ("api-ms-win-core-heap-l1-1-0.dll", &[
        "HeapSize",
        "HeapSetInformation",
        "HeapReAlloc",
        "HeapAlloc",
        "GetProcessHeap",
        "HeapFree",
    ]),
    ("api-ms-win-core-errorhandling-l1-1-0.dll", &[
        "SetUnhandledExceptionFilter",
        "SetErrorMode",
        "UnhandledExceptionFilter",
        "GetLastError",
        "SetLastError",
    ]),
    ("api-ms-win-core-threadpool-l1-2-0.dll", &[
        "CreateThreadpoolTimer",
        "SetThreadpoolTimer",
        "CloseThreadpoolTimer",
        "WaitForThreadpoolTimerCallbacks",
    ]),
    ("api-ms-win-core-processthreads-l1-1-0.dll", &[
        "GetCurrentThreadId",
        "OpenThread",
        "GetCurrentProcessId",
        "GetCurrentProcess",
        "GetExitCodeProcess",
        "InitializeProcThreadAttributeList",
        "ResumeThread",
        "UpdateProcThreadAttribute",
        "DeleteProcThreadAttributeList",
        "GetStartupInfoW",
        "CreateProcessAsUserW",
        "CreateProcessW",
        "TerminateProcess",
    ]),
    ("api-ms-win-core-localization-l1-2-0.dll", &[
        "GetACP",
        "GetLocaleInfoW",
        "FormatMessageW",
        "GetThreadLocale",
        "GetCPInfo",
        "SetThreadLocale",
        "GetUserDefaultLCID",
    ]),
    ("api-ms-win-core-debug-l1-1-0.dll", &[
        "OutputDebugStringW",
        "DebugBreak",
        "IsDebuggerPresent",
    ]),
    ("api-ms-win-core-handle-l1-1-0.dll", &[
        "CloseHandle",
        "DuplicateHandle",
    ]),
    ("api-ms-win-core-memory-l1-1-0.dll", &[
        "VirtualQuery",
        "ReadProcessMemory",
        "VirtualFree",
        "VirtualAlloc",
    ]),
    ("api-ms-win-core-console-l1-1-0.dll", &[
        "WriteConsoleW",
        "GetConsoleOutputCP",
        "ReadConsoleW",
        "GetConsoleMode",
        "SetConsoleMode",
        "SetConsoleCtrlHandler",
    ]),
    ("api-ms-win-core-file-l1-1-0.dll", &[
        "GetDiskFreeSpaceExW",
        "FindFirstFileExW",
        "GetFullPathNameW",
        "FindFirstFileW",
        "CreateDirectoryW",
        "GetFileAttributesExW",
        "GetDriveTypeW",
        "FindClose",
        "SetFileAttributesW",
        "GetVolumeInformationW",
        "CreateFileW",
        "ReadFile",
        "SetFilePointerEx",
        "WriteFile",
        "GetFileSize",
        "SetEndOfFile",
        "GetFileType",
        "FileTimeToLocalFileTime",
        "DeleteFileW",
        "SetFileTime",
        "GetFileAttributesW",
        "FlushFileBuffers",
        "RemoveDirectoryW",
        "CompareFileTime",
        "FindNextFileW",
        "GetVolumePathNameW",
        "SetFilePointer",
    ]),
    ("api-ms-win-core-string-l1-1-0.dll", &[
        "MultiByteToWideChar",
        "CompareStringOrdinal",
        "WideCharToMultiByte",
    ]),
    ("api-ms-win-core-processenvironment-l1-1-0.dll", &[
        "GetStdHandle",
        "GetEnvironmentVariableW",
        "GetCommandLineW",
        "GetEnvironmentStringsW",
        "FreeEnvironmentStringsW",
        "SetEnvironmentVariableW",
        "SetEnvironmentStringsW",
        "SearchPathW",
        "ExpandEnvironmentStringsW",
        "SetCurrentDirectoryW",
        "GetCurrentDirectoryW",
    ]),
    ("api-ms-win-core-console-l2-1-0.dll", &[
        "SetConsoleCursorPosition",
        "ScrollConsoleScreenBufferW",
        "FillConsoleOutputAttribute",
        "SetConsoleTextAttribute",
        "GetConsoleScreenBufferInfo",
        "FlushConsoleInputBuffer",
        "FillConsoleOutputCharacterW",
    ]),
    ("api-ms-win-security-base-l1-1-0.dll", &[
        "GetSecurityDescriptorOwner",
        "GetFileSecurityW",
        "RevertToSelf",
    ]),
    ("api-ms-win-core-sysinfo-l1-1-0.dll", &[
        "SetLocalTime",
        "GetSystemTime",
        "GetLocalTime",
        "GetWindowsDirectoryW",
        "GetSystemTimeAsFileTime",
        "GetVersion",
    ]),
    ("api-ms-win-core-timezone-l1-1-0.dll", &[
        "FileTimeToSystemTime",
        "SystemTimeToFileTime",
    ]),
    ("api-ms-win-core-datetime-l1-1-0.dll", &[
        "GetDateFormatW",
        "GetTimeFormatW",
    ]),
    ("api-ms-win-core-systemtopology-l1-1-0.dll", &[
        "GetNumaNodeProcessorMaskEx",
        "GetNumaHighestNodeNumber",
    ]),
    ("api-ms-win-core-console-l2-2-0.dll", &[
        "SetConsoleTitleW",
        "GetConsoleTitleW",
    ]),
    ("api-ms-win-core-registry-l1-1-0.dll", &[
        "RegDeleteKeyExW",
        "RegQueryValueExW",
        "RegGetValueW",
        "RegEnumKeyExW",
        "RegCreateKeyExW",
        "RegDeleteValueW",
        "RegCloseKey",
        "RegOpenKeyExW",
        "RegSetValueExW",
    ]),
    ("api-ms-win-core-processenvironment-l1-2-0.dll", &[
        "NeedCurrentDirectoryForExePathW",
    ]),
    ("api-ms-win-core-file-l2-1-0.dll", &[
        "GetFileInformationByHandleEx",
        "MoveFileExW",
        "CreateHardLinkW",
        "CreateSymbolicLinkW",
        "MoveFileWithProgressW",
    ]),
    ("api-ms-win-core-heap-l2-1-0.dll", &[
        "GlobalFree",
        "GlobalAlloc",
        "LocalFree",
    ]),
    ("api-ms-win-core-file-l2-1-2.dll", &[
        "CopyFileW",
    ]),
    ("api-ms-win-core-io-l1-1-0.dll", &[
        "DeviceIoControl",
    ]),
    ("api-ms-win-core-console-l3-2-0.dll", &[
        "GetConsoleWindow",
    ]),
    ("api-ms-win-core-processtopology-l1-1-0.dll", &[
        "GetThreadGroupAffinity",
    ]),
    ("api-ms-win-eventing-provider-l1-1-0.dll", &[
        "EventWriteTransfer",
        "EventUnregister",
        "EventRegister",
        "EventSetInformation",
    ]),
    ("api-ms-win-core-synch-l1-2-0.dll", &[
        "InitOnceComplete",
        "InitOnceBeginInitialize",
    ]),
    ("api-ms-win-core-processthreads-l1-1-1.dll", &[
        "IsProcessorFeaturePresent",
    ]),
    ("api-ms-win-core-profile-l1-1-0.dll", &[
        "QueryPerformanceCounter",
    ]),
    ("api-ms-win-core-interlocked-l1-1-0.dll", &[
        "InitializeSListHead",
    ]),
    ("api-ms-win-core-misc-l1-1-0.dll", &[
        "lstrcmpW",
        "lstrcmpiW",
    ]),
    ("api-ms-win-core-apiquery-l1-1-0.dll", &[
        "ApiSetQueryApiSetPresence",
    ]),
    ("api-ms-win-core-delayload-l1-1-1.dll", &[
        "ResolveDelayLoadedAPI",
    ]),
    ("api-ms-win-core-delayload-l1-1-0.dll", &[
        "DelayLoadFailureHook",
    ]),
];
