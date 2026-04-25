#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub const ANYCODE_PLUGIN_SDK_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginStatus {
    Ok = 0,
    Error = 1,
    UnsupportedAbi = 2,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginCapability {
    Language = 1,
    Command = 2,
    Panel = 3,
    CodeAction = 4,
    Formatter = 5,
    Test = 6,
    AiTool = 7,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PluginHostApi {
    pub abi_version: u32,
    pub host_context: u64,
    pub register_json: Option<
        extern "C" fn(host_context: u64, json_ptr: *const u8, json_len: usize) -> PluginStatus,
    >,
    pub log:
        Option<extern "C" fn(host_context: u64, level: u32, msg_ptr: *const u8, msg_len: usize)>,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PluginDescriptor {
    pub sdk_version: u32,
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub version_ptr: *const u8,
    pub version_len: usize,
}

#[derive(Clone, Debug)]
pub struct CommandContribution {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct CodeActionContribution {
    pub id: String,
    pub title: String,
    pub language_id: String,
}

#[derive(Clone, Debug)]
pub struct PluginRegistration {
    pub commands: Vec<CommandContribution>,
    pub code_actions: Vec<CodeActionContribution>,
}

impl PluginRegistration {
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            code_actions: Vec::new(),
        }
    }
}

pub type PluginInit = extern "C" fn(host: *const PluginHostApi) -> PluginStatus;
pub type PluginRegister = extern "C" fn(host: *const PluginHostApi) -> PluginStatus;
pub type PluginShutdown = extern "C" fn() -> PluginStatus;
