use alloc::string::String;
use anyos_std::fs;

use crate::config::WxeConfig;

const DLL_MANIFEST: &str = "\
ntdll.dll
kernelbase.dll
kernel32.dll
msvcrt.dll
ucrtbase.dll
vcruntime140.dll
advapi32.dll
shell32.dll
shlwapi.dll
ole32.dll
user32.dll
gdi32.dll
win32u.dll
comctl32.dll
comdlg32.dll
api-ms-win-core-file-l1-1-0.dll
api-ms-win-core-processthreads-l1-1-0.dll
api-ms-win-core-memory-l1-1-0.dll
api-ms-win-core-synch-l1-1-0.dll
api-ms-win-core-console-l1-1-0.dll
api-ms-win-core-errorhandling-l1-1-0.dll
api-ms-win-core-libraryloader-l1-2-0.dll
api-ms-win-core-heap-l1-1-0.dll
api-ms-win-core-timezone-l1-1-0.dll
api-ms-win-core-sysinfo-l1-2-1.dll
api-ms-win-core-profile-l1-1-0.dll
api-ms-win-core-string-l1-1-0.dll
api-ms-win-core-localization-l1-2-0.dll
api-ms-win-core-registry-l1-1-0.dll
api-ms-win-core-handle-l1-1-0.dll
api-ms-win-core-namedpipe-l1-1-0.dll
";

const DLL_ROUTES: &str = "\
# WXE DLL route manifest v1
# format: dll!export=route
ntdll.dll!NtTerminateProcess=nt:0x0001
ntdll.dll!ZwTerminateProcess=nt:0x0001
ntdll.dll!NtTerminateThread=nt:0x0002
ntdll.dll!ZwTerminateThread=nt:0x0002
ntdll.dll!NtClose=nt:0x0003
ntdll.dll!ZwClose=nt:0x0003
ntdll.dll!NtReadFile=nt:0x0004
ntdll.dll!ZwReadFile=nt:0x0004
ntdll.dll!NtWriteFile=nt:0x0005
ntdll.dll!ZwWriteFile=nt:0x0005
ntdll.dll!NtDelayExecution=nt:0x0006
ntdll.dll!ZwDelayExecution=nt:0x0006
ntdll.dll!NtQuerySystemTime=nt:0x0007
ntdll.dll!ZwQuerySystemTime=nt:0x0007
ntdll.dll!NtQueryPerformanceCounter=nt:0x0008
ntdll.dll!ZwQueryPerformanceCounter=nt:0x0008
ntdll.dll!NtAllocateVirtualMemory=nt:0x0009
ntdll.dll!ZwAllocateVirtualMemory=nt:0x0009
ntdll.dll!NtFreeVirtualMemory=nt:0x000a
ntdll.dll!ZwFreeVirtualMemory=nt:0x000a
ntdll.dll!NtProtectVirtualMemory=nt:0x000b
ntdll.dll!ZwProtectVirtualMemory=nt:0x000b
ntdll.dll!NtQueryInformationProcess=nt:0x000c
ntdll.dll!ZwQueryInformationProcess=nt:0x000c
ntdll.dll!NtQueryInformationThread=nt:0x000d
ntdll.dll!ZwQueryInformationThread=nt:0x000d
ntdll.dll!NtCreateFile=nt:0x000e
ntdll.dll!ZwCreateFile=nt:0x000e
ntdll.dll!NtOpenFile=nt:0x000f
ntdll.dll!ZwOpenFile=nt:0x000f
ntdll.dll!NtQueryInformationFile=nt:0x0010
ntdll.dll!ZwQueryInformationFile=nt:0x0010
ntdll.dll!RtlGetCurrentPeb=rtl:peb
ntdll.dll!RtlGetVersion=rtl:version
ntdll.dll!RtlInitUnicodeString=rtl:unicode:init
ntdll.dll!RtlDosPathNameToNtPathName_U=rtl:path:dos-to-nt
ntdll.dll!RtlAllocateHeap=rtl:heap:alloc
ntdll.dll!RtlFreeHeap=rtl:heap:free
ntdll.dll!RtlReAllocateHeap=rtl:heap:realloc
kernelbase.dll!ExitProcess=win32:process:exit
kernelbase.dll!GetCommandLineA=win32:process:cmdline-a
kernelbase.dll!GetCommandLineW=win32:process:cmdline-w
kernelbase.dll!GetEnvironmentStringsA=win32:env:block-a
kernelbase.dll!GetEnvironmentStringsW=win32:env:block-w
kernelbase.dll!FreeEnvironmentStringsA=win32:env:free
kernelbase.dll!FreeEnvironmentStringsW=win32:env:free
kernelbase.dll!GetCurrentProcess=win32:pseudo:process
kernelbase.dll!GetCurrentThread=win32:pseudo:thread
kernelbase.dll!GetCurrentProcessId=win32:id:process
kernelbase.dll!GetCurrentThreadId=win32:id:thread
kernelbase.dll!GetModuleHandleA=win32:module:handle-a
kernelbase.dll!GetModuleHandleW=win32:module:handle-w
kernelbase.dll!GetProcAddress=win32:module:proc
kernelbase.dll!LoadLibraryA=win32:module:load-a
kernelbase.dll!LoadLibraryW=win32:module:load-w
kernelbase.dll!LoadLibraryExW=win32:module:load-ex-w
kernelbase.dll!GetStdHandle=win32:console:std-handle
kernelbase.dll!SetStdHandle=win32:console:set-std-handle
kernelbase.dll!CreateFileA=win32:file:create-a
kernelbase.dll!CreateFileW=win32:file:create-w
kernelbase.dll!ReadFile=win32:file:read
kernelbase.dll!WriteFile=win32:file:write
kernelbase.dll!CloseHandle=win32:handle:close
kernelbase.dll!GetFileType=win32:file:type
kernelbase.dll!GetFileAttributesA=win32:file:attributes-a
kernelbase.dll!GetFileAttributesW=win32:file:attributes-w
kernelbase.dll!GetFileSizeEx=win32:file:size-ex
kernelbase.dll!SetFilePointerEx=win32:file:pointer-ex
kernelbase.dll!Sleep=win32:time:sleep
kernelbase.dll!QueryPerformanceCounter=win32:time:qpc
kernelbase.dll!QueryPerformanceFrequency=win32:time:qpf
kernelbase.dll!GetSystemTimeAsFileTime=win32:time:filetime
kernelbase.dll!VirtualAlloc=win32:memory:alloc
kernelbase.dll!VirtualFree=win32:memory:free
kernelbase.dll!VirtualProtect=win32:memory:protect
kernelbase.dll!HeapCreate=win32:heap:create
kernelbase.dll!HeapDestroy=win32:heap:destroy
kernelbase.dll!HeapAlloc=win32:heap:alloc
kernelbase.dll!HeapFree=win32:heap:free
kernelbase.dll!HeapReAlloc=win32:heap:realloc
kernelbase.dll!GetProcessHeap=win32:heap:process
kernelbase.dll!GetLastError=win32:last-error:get
kernelbase.dll!SetLastError=win32:last-error:set
kernelbase.dll!TlsAlloc=win32:tls:alloc
kernelbase.dll!TlsFree=win32:tls:free
kernelbase.dll!TlsGetValue=win32:tls:get
kernelbase.dll!TlsSetValue=win32:tls:set
kernelbase.dll!GetConsoleMode=win32:console:get-mode
kernelbase.dll!SetConsoleMode=win32:console:set-mode
kernelbase.dll!ReadConsoleA=win32:console:read-a
kernelbase.dll!ReadConsoleW=win32:console:read-w
kernelbase.dll!WriteConsoleA=win32:console:write-a
kernelbase.dll!WriteConsoleW=win32:console:write-w
kernel32.dll!ExitProcess=forward:kernelbase.dll!ExitProcess
kernel32.dll!GetCommandLineA=forward:kernelbase.dll!GetCommandLineA
kernel32.dll!GetCommandLineW=forward:kernelbase.dll!GetCommandLineW
kernel32.dll!GetEnvironmentStringsA=forward:kernelbase.dll!GetEnvironmentStringsA
kernel32.dll!GetEnvironmentStringsW=forward:kernelbase.dll!GetEnvironmentStringsW
kernel32.dll!FreeEnvironmentStringsA=forward:kernelbase.dll!FreeEnvironmentStringsA
kernel32.dll!FreeEnvironmentStringsW=forward:kernelbase.dll!FreeEnvironmentStringsW
kernel32.dll!GetCurrentProcess=forward:kernelbase.dll!GetCurrentProcess
kernel32.dll!GetCurrentThread=forward:kernelbase.dll!GetCurrentThread
kernel32.dll!GetCurrentProcessId=forward:kernelbase.dll!GetCurrentProcessId
kernel32.dll!GetCurrentThreadId=forward:kernelbase.dll!GetCurrentThreadId
kernel32.dll!GetModuleHandleA=forward:kernelbase.dll!GetModuleHandleA
kernel32.dll!GetModuleHandleW=forward:kernelbase.dll!GetModuleHandleW
kernel32.dll!GetProcAddress=forward:kernelbase.dll!GetProcAddress
kernel32.dll!LoadLibraryA=forward:kernelbase.dll!LoadLibraryA
kernel32.dll!LoadLibraryW=forward:kernelbase.dll!LoadLibraryW
kernel32.dll!LoadLibraryExW=forward:kernelbase.dll!LoadLibraryExW
kernel32.dll!GetStdHandle=forward:kernelbase.dll!GetStdHandle
kernel32.dll!SetStdHandle=forward:kernelbase.dll!SetStdHandle
kernel32.dll!CreateFileA=forward:kernelbase.dll!CreateFileA
kernel32.dll!CreateFileW=forward:kernelbase.dll!CreateFileW
kernel32.dll!ReadFile=forward:kernelbase.dll!ReadFile
kernel32.dll!WriteFile=forward:kernelbase.dll!WriteFile
kernel32.dll!CloseHandle=forward:kernelbase.dll!CloseHandle
kernel32.dll!GetFileType=forward:kernelbase.dll!GetFileType
kernel32.dll!GetFileAttributesA=forward:kernelbase.dll!GetFileAttributesA
kernel32.dll!GetFileAttributesW=forward:kernelbase.dll!GetFileAttributesW
kernel32.dll!GetFileSizeEx=forward:kernelbase.dll!GetFileSizeEx
kernel32.dll!SetFilePointerEx=forward:kernelbase.dll!SetFilePointerEx
kernel32.dll!Sleep=forward:kernelbase.dll!Sleep
kernel32.dll!QueryPerformanceCounter=forward:kernelbase.dll!QueryPerformanceCounter
kernel32.dll!QueryPerformanceFrequency=forward:kernelbase.dll!QueryPerformanceFrequency
kernel32.dll!GetSystemTimeAsFileTime=forward:kernelbase.dll!GetSystemTimeAsFileTime
kernel32.dll!VirtualAlloc=forward:kernelbase.dll!VirtualAlloc
kernel32.dll!VirtualFree=forward:kernelbase.dll!VirtualFree
kernel32.dll!VirtualProtect=forward:kernelbase.dll!VirtualProtect
kernel32.dll!HeapCreate=forward:kernelbase.dll!HeapCreate
kernel32.dll!HeapDestroy=forward:kernelbase.dll!HeapDestroy
kernel32.dll!HeapAlloc=forward:kernelbase.dll!HeapAlloc
kernel32.dll!HeapFree=forward:kernelbase.dll!HeapFree
kernel32.dll!HeapReAlloc=forward:kernelbase.dll!HeapReAlloc
kernel32.dll!GetProcessHeap=forward:kernelbase.dll!GetProcessHeap
kernel32.dll!GetLastError=forward:kernelbase.dll!GetLastError
kernel32.dll!SetLastError=forward:kernelbase.dll!SetLastError
kernel32.dll!TlsAlloc=forward:kernelbase.dll!TlsAlloc
kernel32.dll!TlsFree=forward:kernelbase.dll!TlsFree
kernel32.dll!TlsGetValue=forward:kernelbase.dll!TlsGetValue
kernel32.dll!TlsSetValue=forward:kernelbase.dll!TlsSetValue
kernel32.dll!GetConsoleMode=forward:kernelbase.dll!GetConsoleMode
kernel32.dll!SetConsoleMode=forward:kernelbase.dll!SetConsoleMode
kernel32.dll!ReadConsoleA=forward:kernelbase.dll!ReadConsoleA
kernel32.dll!ReadConsoleW=forward:kernelbase.dll!ReadConsoleW
kernel32.dll!WriteConsoleA=forward:kernelbase.dll!WriteConsoleA
kernel32.dll!WriteConsoleW=forward:kernelbase.dll!WriteConsoleW
api-ms-win-core-file-l1-1-0.dll!ReadFile=forward:kernelbase.dll!ReadFile
api-ms-win-core-file-l1-1-0.dll!WriteFile=forward:kernelbase.dll!WriteFile
api-ms-win-core-file-l1-1-0.dll!CloseHandle=forward:kernelbase.dll!CloseHandle
api-ms-win-core-file-l1-1-0.dll!GetFileType=forward:kernelbase.dll!GetFileType
api-ms-win-core-file-l1-1-0.dll!CreateFileA=forward:kernelbase.dll!CreateFileA
api-ms-win-core-file-l1-1-0.dll!CreateFileW=forward:kernelbase.dll!CreateFileW
api-ms-win-core-file-l1-1-0.dll!GetFileAttributesA=forward:kernelbase.dll!GetFileAttributesA
api-ms-win-core-file-l1-1-0.dll!GetFileAttributesW=forward:kernelbase.dll!GetFileAttributesW
api-ms-win-core-file-l1-1-0.dll!GetFileSizeEx=forward:kernelbase.dll!GetFileSizeEx
api-ms-win-core-file-l1-1-0.dll!SetFilePointerEx=forward:kernelbase.dll!SetFilePointerEx
api-ms-win-core-processthreads-l1-1-0.dll!ExitProcess=forward:kernelbase.dll!ExitProcess
api-ms-win-core-memory-l1-1-0.dll!VirtualAlloc=forward:kernelbase.dll!VirtualAlloc
api-ms-win-core-memory-l1-1-0.dll!VirtualFree=forward:kernelbase.dll!VirtualFree
api-ms-win-core-memory-l1-1-0.dll!VirtualProtect=forward:kernelbase.dll!VirtualProtect
api-ms-win-core-synch-l1-1-0.dll!Sleep=forward:kernelbase.dll!Sleep
api-ms-win-core-console-l1-1-0.dll!GetStdHandle=forward:kernelbase.dll!GetStdHandle
api-ms-win-core-console-l1-1-0.dll!ReadConsoleW=forward:kernelbase.dll!ReadConsoleW
api-ms-win-core-console-l1-1-0.dll!WriteConsoleW=forward:kernelbase.dll!WriteConsoleW
api-ms-win-core-errorhandling-l1-1-0.dll!GetLastError=forward:kernelbase.dll!GetLastError
api-ms-win-core-errorhandling-l1-1-0.dll!SetLastError=forward:kernelbase.dll!SetLastError
api-ms-win-core-libraryloader-l1-2-0.dll!GetModuleHandleW=forward:kernelbase.dll!GetModuleHandleW
api-ms-win-core-libraryloader-l1-2-0.dll!GetProcAddress=forward:kernelbase.dll!GetProcAddress
api-ms-win-core-libraryloader-l1-2-0.dll!LoadLibraryW=forward:kernelbase.dll!LoadLibraryW
api-ms-win-core-libraryloader-l1-2-0.dll!LoadLibraryExW=forward:kernelbase.dll!LoadLibraryExW
api-ms-win-core-heap-l1-1-0.dll!GetProcessHeap=forward:kernelbase.dll!GetProcessHeap
api-ms-win-core-heap-l1-1-0.dll!HeapAlloc=forward:kernelbase.dll!HeapAlloc
api-ms-win-core-heap-l1-1-0.dll!HeapFree=forward:kernelbase.dll!HeapFree
api-ms-win-core-profile-l1-1-0.dll!QueryPerformanceCounter=forward:kernelbase.dll!QueryPerformanceCounter
api-ms-win-core-profile-l1-1-0.dll!QueryPerformanceFrequency=forward:kernelbase.dll!QueryPerformanceFrequency
";

const UI_ROUTES: &str = "\
# WXE UI route manifest v1
# format: dll!export=route
user32.dll!MessageBoxA=anyui:dialog:message-box-a
user32.dll!MessageBoxW=anyui:dialog:message-box-w
user32.dll!GetDesktopWindow=wxe-ui:pseudo:desktop-window
user32.dll!GetDC=anyui:dc:get
user32.dll!ReleaseDC=anyui:dc:release
user32.dll!GetSystemMetrics=anyui:profile:system-metrics
user32.dll!RegisterClassA=wxe-ui:class:register-a
user32.dll!RegisterClassW=wxe-ui:class:register-w
user32.dll!RegisterClassExA=wxe-ui:class:register-ex-a
user32.dll!RegisterClassExW=wxe-ui:class:register-ex-w
user32.dll!CreateWindowExA=anyui:window:create-a
user32.dll!CreateWindowExW=anyui:window:create-w
user32.dll!DestroyWindow=anyui:window:destroy
user32.dll!DefWindowProcA=wxe-ui:window:defproc-a
user32.dll!DefWindowProcW=wxe-ui:window:defproc-w
user32.dll!ShowWindow=anyui:window:show
user32.dll!UpdateWindow=anyui:window:update
user32.dll!GetMessageA=anyui:message:get-a
user32.dll!GetMessageW=anyui:message:get-w
user32.dll!PeekMessageA=anyui:message:peek-a
user32.dll!PeekMessageW=anyui:message:peek-w
user32.dll!TranslateMessage=wxe-ui:message:translate
user32.dll!DispatchMessageA=anyui:message:dispatch-a
user32.dll!DispatchMessageW=anyui:message:dispatch-w
user32.dll!PostQuitMessage=wxe-ui:message:post-quit
user32.dll!PostMessageA=wxe-ui:message:post-a
user32.dll!PostMessageW=wxe-ui:message:post-w
user32.dll!SendMessageA=wxe-ui:message:send-a
user32.dll!SendMessageW=wxe-ui:message:send-w
user32.dll!SetTimer=wxe-ui:timer:set
user32.dll!KillTimer=wxe-ui:timer:kill
gdi32.dll!CreateCompatibleDC=anyui:gdi:dc:create-compatible
gdi32.dll!DeleteDC=anyui:gdi:dc:delete
gdi32.dll!SaveDC=wxe-ui:gdi:dc:save
gdi32.dll!RestoreDC=wxe-ui:gdi:dc:restore
gdi32.dll!GetStockObject=wxe-ui:gdi:object:stock
gdi32.dll!SelectObject=wxe-ui:gdi:object:select
gdi32.dll!DeleteObject=wxe-ui:gdi:object:delete
gdi32.dll!CreateSolidBrush=wxe-ui:gdi:brush:solid
gdi32.dll!CreatePen=wxe-ui:gdi:pen:create
gdi32.dll!SetTextColor=wxe-ui:gdi:text:color
gdi32.dll!SetBkColor=wxe-ui:gdi:background:color
gdi32.dll!SetBkMode=wxe-ui:gdi:background:mode
gdi32.dll!TextOutA=anyui:gdi:text:out-a
gdi32.dll!TextOutW=anyui:gdi:text:out-w
gdi32.dll!ExtTextOutA=anyui:gdi:text:ext-out-a
gdi32.dll!ExtTextOutW=anyui:gdi:text:ext-out-w
gdi32.dll!GetTextExtentPoint32A=anyui:gdi:text:extent-a
gdi32.dll!GetTextExtentPoint32W=anyui:gdi:text:extent-w
gdi32.dll!CreateFontIndirectA=wxe-ui:gdi:font:create-a
gdi32.dll!CreateFontIndirectW=wxe-ui:gdi:font:create-w
gdi32.dll!Rectangle=anyui:gdi:shape:rectangle
gdi32.dll!PatBlt=anyui:gdi:blit:pat
gdi32.dll!BitBlt=anyui:gdi:blit:bit
win32u.dll!NtUserCreateWindowEx=anyui-kernel:user:create-window
win32u.dll!NtUserDestroyWindow=anyui-kernel:user:destroy-window
win32u.dll!NtUserGetMessage=anyui-kernel:user:get-message
win32u.dll!NtUserPeekMessage=anyui-kernel:user:peek-message
win32u.dll!NtUserDispatchMessage=anyui-kernel:user:dispatch-message
win32u.dll!NtUserMessageCall=anyui-kernel:user:message-call
win32u.dll!NtUserGetDC=anyui-kernel:user:get-dc
win32u.dll!NtUserReleaseDC=anyui-kernel:user:release-dc
win32u.dll!NtGdiCreateCompatibleDC=anyui-kernel:gdi:create-compatible-dc
win32u.dll!NtGdiDeleteObjectApp=anyui-kernel:gdi:delete-object
win32u.dll!NtGdiSelectBrush=anyui-kernel:gdi:select-brush
win32u.dll!NtGdiPatBlt=anyui-kernel:gdi:pat-blt
win32u.dll!NtGdiBitBlt=anyui-kernel:gdi:bit-blt
";

const ANYUI_BINDINGS: &str = "\
# WXE libanyui binding manifest v1
# format: wxe-route=libanyui-export[,notes]
anyui:init=libanyui.so!anyui_init
anyui:window:create=libanyui.so!anyui_create_window
anyui:window:destroy=libanyui.so!anyui_destroy_window
anyui:window:resize=libanyui.so!anyui_resize_window
anyui:window:move=libanyui.so!anyui_move_window
anyui:window:minimize=libanyui.so!anyui_minimize_window
anyui:window:show=libanyui.so!anyui_set_visible
anyui:window:update=libanyui.so!anyui_flush_display
anyui:control:create=libanyui.so!anyui_create_control
anyui:control:add=libanyui.so!anyui_add_control
anyui:control:add-child=libanyui.so!anyui_add_child
anyui:control:remove=libanyui.so!anyui_remove
anyui:control:remove-child=libanyui.so!anyui_remove_child
anyui:control:set-text=libanyui.so!anyui_set_text
anyui:control:set-position=libanyui.so!anyui_set_position
anyui:control:set-size=libanyui.so!anyui_set_size
anyui:control:set-visible=libanyui.so!anyui_set_visible
anyui:control:set-color=libanyui.so!anyui_set_color
anyui:control:set-style=libanyui.so!anyui_set_style
anyui:control:set-state=libanyui.so!anyui_set_state
anyui:message-box=libanyui.so!anyui_message_box
anyui:event:on-event=libanyui.so!anyui_on_event
anyui:event:run-once=libanyui.so!anyui_run_once
anyui:event:run=libanyui.so!anyui_run
anyui:event:quit=libanyui.so!anyui_quit
anyui:canvas:create=libanyui.so!anyui_create_control,KIND_CANVAS
anyui:canvas:clear=libanyui.so!anyui_canvas_clear
anyui:canvas:fill-rect=libanyui.so!anyui_canvas_fill_rect
anyui:canvas:draw-rect=libanyui.so!anyui_canvas_draw_rect
anyui:canvas:draw-line=libanyui.so!anyui_canvas_draw_line
anyui:canvas:draw-text=libanyui.so!anyui_canvas_draw_text
anyui:canvas:copy-from=libanyui.so!anyui_canvas_copy_from
anyui:canvas:copy-to=libanyui.so!anyui_canvas_copy_to
anyui:canvas:buffer=libanyui.so!anyui_canvas_get_buffer
anyui:canvas:stride=libanyui.so!anyui_canvas_get_stride
anyui:text:measure=libanyui.so!anyui_measure_text
";

const NT_SERVICES: &str = "\
# WXE NT service profile v1
# id,name,status
0x0001,NtTerminateProcess,implemented-current-process
0x0002,NtTerminateThread,implemented-current-thread
0x0003,NtClose,implemented-fd-backed
0x0004,NtReadFile,implemented-fd-backed
0x0005,NtWriteFile,implemented-fd-backed
0x0006,NtDelayExecution,implemented-relative-timeout
0x0007,NtQuerySystemTime,implemented
0x0008,NtQueryPerformanceCounter,implemented
0x0009,NtAllocateVirtualMemory,implemented-anonymous
0x000a,NtFreeVirtualMemory,implemented-anonymous
0x000b,NtProtectVirtualMemory,implemented-validate-noop
0x000c,NtQueryInformationProcess,implemented-basic-info
0x000d,NtQueryInformationThread,implemented-basic-info
0x000e,NtCreateFile,implemented-wxe-path-fd-backed
0x000f,NtOpenFile,implemented-wxe-path-fd-backed
0x0010,NtQueryInformationFile,implemented-standard-info
0x0100,WxeGetModuleHandleA,private-win32-helper
0x0101,WxeGetModuleHandleW,private-win32-helper
0x0102,WxeGetProcAddress,private-win32-helper
0x0103,WxeLoadLibraryA,private-win32-helper-loaded-modules-only
0x0104,WxeLoadLibraryW,private-win32-helper-loaded-modules-only
0x0105,WxeCreateFileA,private-win32-helper
0x0106,WxeCreateFileW,private-win32-helper
0x0107,WxeGetFileAttributesA,private-win32-helper
0x0108,WxeGetFileAttributesW,private-win32-helper
0x0109,WxeGetFileSizeEx,private-win32-helper
0x010a,WxeSetFilePointerEx,private-win32-helper
";

const EXPECTED_DLLS: &[&str] = &[
    "ntdll.dll",
    "kernelbase.dll",
    "kernel32.dll",
    "msvcrt.dll",
    "ucrtbase.dll",
    "vcruntime140.dll",
    "advapi32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "ole32.dll",
    "user32.dll",
    "gdi32.dll",
    "win32u.dll",
    "comctl32.dll",
    "comdlg32.dll",
    "api-ms-win-core-file-l1-1-0.dll",
    "api-ms-win-core-processthreads-l1-1-0.dll",
    "api-ms-win-core-memory-l1-1-0.dll",
    "api-ms-win-core-synch-l1-1-0.dll",
    "api-ms-win-core-console-l1-1-0.dll",
    "api-ms-win-core-errorhandling-l1-1-0.dll",
    "api-ms-win-core-libraryloader-l1-2-0.dll",
    "api-ms-win-core-heap-l1-1-0.dll",
    "api-ms-win-core-timezone-l1-1-0.dll",
    "api-ms-win-core-sysinfo-l1-2-1.dll",
    "api-ms-win-core-profile-l1-1-0.dll",
    "api-ms-win-core-string-l1-1-0.dll",
    "api-ms-win-core-localization-l1-2-0.dll",
    "api-ms-win-core-registry-l1-1-0.dll",
    "api-ms-win-core-handle-l1-1-0.dll",
    "api-ms-win-core-namedpipe-l1-1-0.dll",
];

pub fn ensure_rootfs_layout(config: &WxeConfig) -> bool {
    let mut ok = true;
    for dir in [
        config.root.as_str(),
        config.drive_c.as_str(),
        config.cache.as_str(),
        config.db.as_str(),
    ] {
        if !ensure_dir(dir) {
            ok = false;
        }
    }
    for dir in [
        join(&config.drive_c, "Windows"),
        join(&config.drive_c, "Windows/System32"),
        join(&config.drive_c, "Windows/Temp"),
        join(&config.drive_c, "Users"),
        join(&config.drive_c, "Users/Default"),
        join(&config.drive_c, "Program Files"),
        join(&config.drive_c, "ProgramData"),
    ] {
        if !ensure_dir(&dir) {
            ok = false;
        }
    }

    let manifest = join(&config.db, "dll-manifest");
    if fs::write_bytes(&manifest, DLL_MANIFEST.as_bytes()).is_err() {
        ok = false;
    }

    if fs::write_bytes(&dll_routes_path(config), DLL_ROUTES.as_bytes()).is_err() {
        ok = false;
    }
    if fs::write_bytes(&ui_routes_path(config), UI_ROUTES.as_bytes()).is_err() {
        ok = false;
    }
    if fs::write_bytes(&anyui_bindings_path(config), ANYUI_BINDINGS.as_bytes()).is_err() {
        ok = false;
    }
    let nt_services = alloc::format!("profile={}\n{}", config.nt_profile, NT_SERVICES);
    if fs::write_bytes(&nt_services_path(config), nt_services.as_bytes()).is_err() {
        ok = false;
    }

    let drive_map = alloc::format!(
        "C={}\nZ={}\n",
        config.drive_c,
        if config.drive_z.is_empty() {
            "<disabled>"
        } else {
            config.drive_z.as_str()
        }
    );
    if fs::write_bytes(&join(&config.db, "drive-map"), drive_map.as_bytes()).is_err() {
        ok = false;
    }

    if !install_wxe_dlls(config) {
        ok = false;
    }

    ok
}

pub fn path_exists(path: &str) -> bool {
    fs::stat(path, &mut [0u32; 7]) == 0
}

pub fn installed_dll_path(config: &WxeConfig, name: &str) -> String {
    join(&config.system32(), name)
}

pub fn expected_dlls() -> &'static [&'static str] {
    EXPECTED_DLLS
}

pub fn install_wxe_dlls(config: &WxeConfig) -> bool {
    let mut ok = true;
    for (idx, dll) in EXPECTED_DLLS.iter().enumerate() {
        let image = crate::wxedll::build_profile_dll(dll, &[DLL_ROUTES, UI_ROUTES], idx as u32 + 1);
        if fs::write_bytes(&installed_dll_path(config, dll), &image).is_err() {
            ok = false;
        }
    }
    ok
}

pub fn dll_routes_path(config: &WxeConfig) -> String {
    join(&config.db, "dll-routes")
}

pub fn ui_routes_path(config: &WxeConfig) -> String {
    join(&config.db, "ui-routes")
}

pub fn anyui_bindings_path(config: &WxeConfig) -> String {
    join(&config.db, "anyui-bindings")
}

pub fn nt_services_path(config: &WxeConfig) -> String {
    join(&config.db, "nt-services")
}

pub fn dll_route_count() -> usize {
    route_count(DLL_ROUTES)
}

pub fn ui_route_count() -> usize {
    route_count(UI_ROUTES)
}

pub fn anyui_binding_count() -> usize {
    route_count(ANYUI_BINDINGS)
}

pub fn nt_service_count() -> usize {
    route_count(NT_SERVICES)
}

fn ensure_dir(path: &str) -> bool {
    path_exists(path) || fs::mkdir(path) == 0 || path_exists(path)
}

fn route_count(manifest: &str) -> usize {
    manifest
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count()
}

fn join(base: &str, rel: &str) -> String {
    if base.ends_with('/') {
        alloc::format!("{}{}", base, rel)
    } else {
        alloc::format!("{}/{}", base, rel)
    }
}
