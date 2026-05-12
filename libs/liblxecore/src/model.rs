use alloc::string::String;

#[derive(Clone)]
pub struct PackageInfo {
    pub package: String,
    pub version: String,
    pub filename: String,
    pub size: usize,
    pub md5: String,
    pub depends: String,
    pub pre_depends: String,
}

pub struct Elf64Diag {
    pub entry: u64,
    pub is_dyn: bool,
    pub interp_path: Option<String>,
}
