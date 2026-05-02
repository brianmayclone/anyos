#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UvHandleKind {
    Unknown = 0,
    Tcp = 1,
    TcpServer = 2,
}
