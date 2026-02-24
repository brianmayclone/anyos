//! SMB2 network filesystem client -- mounts remote SMB/CIFS shares via TCP.
//!
//! Implements a minimal SMB2 (dialect 0x0202) client that connects over TCP port 445,
//! performs negotiate + session setup (anonymous/guest) + tree connect, then provides
//! standard Filesystem trait operations (lookup, read, write, readdir, create, delete).

use crate::fs::file::{DirEntry, FileType};
use crate::fs::vfs::FsError;
use crate::net::tcp;
use crate::net::types::Ipv4Addr;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// SMB2 Protocol Constants
// ---------------------------------------------------------------------------

/// SMB2 protocol magic: 0xFE 'S' 'M' 'B'
const SMB2_MAGIC: [u8; 4] = [0xFE, b'S', b'M', b'B'];

/// SMB2 header size (always 64 bytes).
const SMB2_HEADER_SIZE: usize = 64;

// SMB2 command codes
const SMB2_NEGOTIATE: u16 = 0;
const SMB2_SESSION_SETUP: u16 = 1;
const SMB2_TREE_CONNECT: u16 = 3;
const SMB2_CREATE: u16 = 5;
const SMB2_CLOSE: u16 = 6;
const SMB2_READ: u16 = 8;
const SMB2_WRITE: u16 = 9;
const SMB2_QUERY_DIRECTORY: u16 = 14;
const SMB2_SET_INFO: u16 = 17;

// SMB2 status codes
const STATUS_SUCCESS: u32 = 0;
const STATUS_MORE_PROCESSING_REQUIRED: u32 = 0xC0000016;
const STATUS_NO_MORE_FILES: u32 = 0x80000006;

// SMB2 CREATE disposition
const FILE_OPEN: u32 = 1;
const FILE_CREATE: u32 = 2;
const FILE_OPEN_IF: u32 = 5;

// SMB2 CREATE options
const FILE_DIRECTORY_FILE: u32 = 0x00000001;
const FILE_NON_DIRECTORY_FILE: u32 = 0x00000040;
const FILE_DELETE_ON_CLOSE: u32 = 0x00001000;

// SMB2 access masks
const GENERIC_READ: u32 = 0x80000000;
const GENERIC_WRITE: u32 = 0x40000000;
const DELETE: u32 = 0x00010000;
const FILE_LIST_DIRECTORY: u32 = 0x00000001;

// SMB2 share access
const FILE_SHARE_READ: u32 = 0x00000001;
const FILE_SHARE_WRITE: u32 = 0x00000002;

// SMB2 file attributes
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x00000010;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x00000080;

// FileInformationClass for QueryDirectory
const FILE_BOTH_DIR_INFORMATION: u8 = 0x03;

// TCP timeout for SMB operations (in PIT ticks, ~5 seconds at 100 Hz)
const SMB_TIMEOUT: u32 = 500;
const SMB_CONNECT_TIMEOUT: u32 = 1000;

// ---------------------------------------------------------------------------
// NTLMSSP Constants for anonymous authentication
// ---------------------------------------------------------------------------

const NTLMSSP_SIGNATURE: &[u8; 8] = b"NTLMSSP\0";
const NTLMSSP_NEGOTIATE: u32 = 1;
const NTLMSSP_AUTH: u32 = 3;

// Negotiate flags for anonymous session
const NTLMSSP_NEGOTIATE_UNICODE: u32 = 0x00000001;
const NTLMSSP_NEGOTIATE_NTLM: u32 = 0x00000200;

// ---------------------------------------------------------------------------
// SmbFs -- the filesystem instance
// ---------------------------------------------------------------------------

/// An open SMB2 file handle cached for inode lookups.
struct SmbHandle {
    /// Inode (path hash) this handle maps to.
    inode: u32,
    /// Original path relative to share root.
    path: String,
}

/// SMB2 network filesystem instance.
pub struct SmbFs {
    /// Kernel TCP socket id.
    socket_id: u32,
    /// SMB2 session id (from session setup).
    session_id: u64,
    /// SMB2 tree id (from tree connect).
    tree_id: u32,
    /// Monotonically increasing message id.
    message_id: u64,
    /// Server IP for logging.
    server_ip: [u8; 4],
    /// Share path (e.g. "//192.168.1.1/share").
    share_path: String,
    /// Inode → path mapping (inode = hash of path).
    path_map: Vec<SmbHandle>,
    /// Max read size from negotiate response.
    max_read_size: u32,
    /// Max write size from negotiate response.
    max_write_size: u32,
}

impl SmbFs {
    /// Connect to an SMB server and mount a share.
    /// `device` format: `//ip_addr/share_name` (e.g. `//192.168.1.1/shared`)
    pub fn connect(device: &str) -> Result<Self, FsError> {
        // Parse device string: //ip/share
        let stripped = device.trim_start_matches('/');
        let (ip_str, share_name) = stripped.split_once('/')
            .ok_or(FsError::InvalidPath)?;

        let ip = parse_ipv4(ip_str).ok_or(FsError::InvalidPath)?;

        crate::serial_println!("[SMBFS] Connecting to {}:{}", ip_str, 445);

        // TCP connect to port 445
        let socket_id = tcp::connect(Ipv4Addr(ip), 445, SMB_CONNECT_TIMEOUT);
        if socket_id == u32::MAX {
            crate::serial_println!("[SMBFS] TCP connect failed");
            return Err(FsError::IoError);
        }

        crate::serial_println!("[SMBFS] TCP connected, socket={}", socket_id);

        let mut fs = SmbFs {
            socket_id,
            session_id: 0,
            tree_id: 0,
            message_id: 0,
            server_ip: ip,
            share_path: String::from(device),
            path_map: Vec::new(),
            max_read_size: 65536,
            max_write_size: 65536,
        };

        // SMB2 Negotiate
        fs.negotiate()?;

        // SMB2 Session Setup (anonymous/guest)
        fs.session_setup()?;

        // SMB2 Tree Connect
        let tree_path = {
            let mut s = String::from("\\\\");
            s.push_str(ip_str);
            s.push('\\');
            s.push_str(share_name);
            s
        };
        fs.tree_connect(&tree_path)?;

        crate::serial_println!("[SMBFS] Mounted //{}:{}/{}", ip_str, 445, share_name);
        Ok(fs)
    }

    /// Disconnect from the server (close TCP).
    pub fn disconnect(self) {
        tcp::close(self.socket_id);
    }

    // -----------------------------------------------------------------------
    // SMB2 Protocol Operations
    // -----------------------------------------------------------------------

    /// Build an SMB2 header for the given command.
    fn build_header(&mut self, command: u16) -> [u8; SMB2_HEADER_SIZE] {
        let mut hdr = [0u8; SMB2_HEADER_SIZE];
        // Protocol ID
        hdr[0..4].copy_from_slice(&SMB2_MAGIC);
        // Structure size = 64
        put_u16_le(&mut hdr[4..6], 64);
        // Credit charge = 1
        put_u16_le(&mut hdr[6..8], 1);
        // Status = 0
        put_u32_le(&mut hdr[8..12], 0);
        // Command
        put_u16_le(&mut hdr[12..14], command);
        // Credit request = 1
        put_u16_le(&mut hdr[14..16], 1);
        // Flags = 0
        put_u32_le(&mut hdr[16..20], 0);
        // NextCommand = 0
        put_u32_le(&mut hdr[20..24], 0);
        // Message ID
        put_u64_le(&mut hdr[24..32], self.message_id);
        self.message_id += 1;
        // Reserved
        put_u32_le(&mut hdr[32..36], 0);
        // Tree ID
        put_u32_le(&mut hdr[36..40], self.tree_id);
        // Session ID
        put_u64_le(&mut hdr[40..48], self.session_id);
        // Signature = 0 (unsigned)
        hdr
    }

    /// Send an SMB2 message (NetBIOS length prefix + header + payload) and receive response.
    fn transact(&mut self, header: &[u8; SMB2_HEADER_SIZE], payload: &[u8]) -> Result<Vec<u8>, FsError> {
        let total_len = SMB2_HEADER_SIZE + payload.len();

        // Build packet: 4-byte NetBIOS length + header + payload
        let mut packet = Vec::with_capacity(4 + total_len);
        // NetBIOS session service: 4-byte big-endian length
        packet.push(0);
        packet.push(((total_len >> 16) & 0xFF) as u8);
        packet.push(((total_len >> 8) & 0xFF) as u8);
        packet.push((total_len & 0xFF) as u8);
        packet.extend_from_slice(header);
        packet.extend_from_slice(payload);

        // Send
        let sent = tcp::send(self.socket_id, &packet, SMB_TIMEOUT);
        if sent == u32::MAX {
            crate::serial_println!("[SMBFS] send failed");
            return Err(FsError::IoError);
        }

        // Receive response: first read 4-byte NetBIOS header
        let mut nb_hdr = [0u8; 4];
        let n = tcp::recv(self.socket_id, &mut nb_hdr, SMB_TIMEOUT);
        if n == u32::MAX || n < 4 {
            crate::serial_println!("[SMBFS] recv NetBIOS header failed (got {})", n);
            return Err(FsError::IoError);
        }

        let resp_len = ((nb_hdr[1] as usize) << 16) | ((nb_hdr[2] as usize) << 8) | (nb_hdr[3] as usize);
        if resp_len == 0 || resp_len > 1024 * 1024 {
            crate::serial_println!("[SMBFS] invalid response length: {}", resp_len);
            return Err(FsError::IoError);
        }

        // Read full response
        let mut response = vec![0u8; resp_len];
        let mut received = 0usize;
        while received < resp_len {
            let n = tcp::recv(self.socket_id, &mut response[received..], SMB_TIMEOUT);
            if n == u32::MAX || n == 0 {
                crate::serial_println!("[SMBFS] recv body failed at {}/{}", received, resp_len);
                return Err(FsError::IoError);
            }
            received += n as usize;
        }

        Ok(response)
    }

    /// Parse response status from an SMB2 response.
    fn response_status(response: &[u8]) -> u32 {
        if response.len() < SMB2_HEADER_SIZE {
            return u32::MAX;
        }
        get_u32_le(&response[8..12])
    }

    /// Extract session_id from a response header.
    fn response_session_id(response: &[u8]) -> u64 {
        if response.len() < SMB2_HEADER_SIZE {
            return 0;
        }
        get_u64_le(&response[40..48])
    }

    /// Extract tree_id from a response header.
    fn response_tree_id(response: &[u8]) -> u32 {
        if response.len() < SMB2_HEADER_SIZE {
            return 0;
        }
        get_u32_le(&response[36..40])
    }

    /// SMB2 Negotiate request.
    fn negotiate(&mut self) -> Result<(), FsError> {
        let hdr = self.build_header(SMB2_NEGOTIATE);

        // Negotiate request body:
        // StructureSize(2)=36, DialectCount(2)=1, SecurityMode(2)=0,
        // Reserved(2)=0, Capabilities(4)=0, ClientGuid(16)=0, ClientStartTime(8)=0,
        // Dialects(2)=0x0202
        let mut body = vec![0u8; 36];
        put_u16_le(&mut body[0..2], 36); // StructureSize
        put_u16_le(&mut body[2..4], 1);  // DialectCount
        // SecurityMode, Reserved, Capabilities, ClientGuid, ClientStartTime are 0
        // Append dialect 0x0202 (SMB 2.0.2)
        body.push(0x02);
        body.push(0x02);

        let resp = self.transact(&hdr, &body)?;
        let status = Self::response_status(&resp);
        if status != STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] Negotiate failed: status=0x{:08X}", status);
            return Err(FsError::IoError);
        }

        // Parse negotiate response for MaxReadSize, MaxWriteSize
        // Response body starts at offset 64 (after header)
        if resp.len() >= SMB2_HEADER_SIZE + 65 {
            let body = &resp[SMB2_HEADER_SIZE..];
            // Offset 32: MaxTransactSize(4), MaxReadSize(4), MaxWriteSize(4)
            if body.len() >= 40 {
                self.max_read_size = get_u32_le(&body[32..36]).min(1024 * 1024);
                self.max_write_size = get_u32_le(&body[36..40]).min(1024 * 1024);
            }
        }

        crate::serial_println!("[SMBFS] Negotiate OK, max_read={}, max_write={}",
            self.max_read_size, self.max_write_size);
        Ok(())
    }

    /// SMB2 Session Setup (anonymous/guest via NTLMSSP).
    fn session_setup(&mut self) -> Result<(), FsError> {
        // Phase 1: Send NTLMSSP_NEGOTIATE
        let negotiate_token = build_ntlmssp_negotiate();
        let mut body1 = vec![0u8; 24];
        put_u16_le(&mut body1[0..2], 25); // StructureSize
        body1[2] = 0; // Flags
        body1[3] = 0; // SecurityMode
        put_u32_le(&mut body1[4..8], 0); // Capabilities
        put_u32_le(&mut body1[8..12], 0); // Channel
        // SecurityBufferOffset = 88 (header 64 + body 24)
        put_u16_le(&mut body1[12..14], 88);
        put_u16_le(&mut body1[14..16], negotiate_token.len() as u16); // SecurityBufferLength
        put_u64_le(&mut body1[16..24], 0); // PreviousSessionId
        body1.extend_from_slice(&negotiate_token);

        let hdr1 = self.build_header(SMB2_SESSION_SETUP);
        let resp1 = self.transact(&hdr1, &body1)?;
        let status1 = Self::response_status(&resp1);

        // Capture session id from first response
        self.session_id = Self::response_session_id(&resp1);

        if status1 != STATUS_MORE_PROCESSING_REQUIRED && status1 != STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] Session setup phase 1 failed: 0x{:08X}", status1);
            return Err(FsError::IoError);
        }

        if status1 == STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] Session setup OK (single phase), session_id={}", self.session_id);
            return Ok(());
        }

        // Phase 2: Send NTLMSSP_AUTH with empty credentials (guest/anonymous)
        let auth_token = build_ntlmssp_auth();
        let mut body2 = vec![0u8; 24];
        put_u16_le(&mut body2[0..2], 25);
        body2[2] = 0;
        body2[3] = 0;
        put_u32_le(&mut body2[4..8], 0);
        put_u32_le(&mut body2[8..12], 0);
        put_u16_le(&mut body2[12..14], 88);
        put_u16_le(&mut body2[14..16], auth_token.len() as u16);
        put_u64_le(&mut body2[16..24], 0);
        body2.extend_from_slice(&auth_token);

        let hdr2 = self.build_header(SMB2_SESSION_SETUP);
        let resp2 = self.transact(&hdr2, &body2)?;
        let status2 = Self::response_status(&resp2);
        self.session_id = Self::response_session_id(&resp2);

        if status2 != STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] Session setup phase 2 failed: 0x{:08X}", status2);
            return Err(FsError::IoError);
        }

        crate::serial_println!("[SMBFS] Session setup OK, session_id={}", self.session_id);
        Ok(())
    }

    /// SMB2 Tree Connect to the specified share path (e.g. `\\server\share`).
    fn tree_connect(&mut self, path: &str) -> Result<(), FsError> {
        let path_u16 = to_utf16le(path);

        let mut body = vec![0u8; 8];
        put_u16_le(&mut body[0..2], 9); // StructureSize
        put_u16_le(&mut body[2..4], 0); // Reserved / Flags
        // PathOffset = 72 (header 64 + body 8)
        put_u16_le(&mut body[4..6], 72);
        put_u16_le(&mut body[6..8], path_u16.len() as u16); // PathLength
        body.extend_from_slice(&path_u16);

        let hdr = self.build_header(SMB2_TREE_CONNECT);
        let resp = self.transact(&hdr, &body)?;
        let status = Self::response_status(&resp);

        if status != STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] Tree connect failed: 0x{:08X}", status);
            return Err(FsError::IoError);
        }

        self.tree_id = Self::response_tree_id(&resp);
        crate::serial_println!("[SMBFS] Tree connect OK, tree_id={}", self.tree_id);
        Ok(())
    }

    /// SMB2 CREATE — open a file or directory on the share.
    /// Returns (file_id_persistent[8], file_id_volatile[8], end_of_file, file_attributes).
    fn smb2_create(
        &mut self,
        path: &str,
        access_mask: u32,
        share_access: u32,
        disposition: u32,
        options: u32,
    ) -> Result<([u8; 16], u64, u32), FsError> {
        // Convert path to UTF-16LE (strip leading /)
        let clean = path.trim_start_matches('/');
        // SMB paths use backslashes
        let smb_path: String = clean.chars().map(|c| if c == '/' { '\\' } else { c }).collect();
        let path_u16 = to_utf16le(&smb_path);

        // CREATE request body: StructureSize=57, then fields
        let mut body = vec![0u8; 56]; // 56 fixed bytes before variable
        put_u16_le(&mut body[0..2], 57); // StructureSize
        body[2] = 0; // SecurityFlags
        body[3] = 0; // RequestedOplockLevel
        put_u32_le(&mut body[4..8], 0); // ImpersonationLevel
        put_u64_le(&mut body[8..16], 0); // SmbCreateFlags
        put_u64_le(&mut body[16..24], 0); // Reserved
        put_u32_le(&mut body[24..28], access_mask); // DesiredAccess
        put_u32_le(&mut body[28..32], FILE_ATTRIBUTE_NORMAL); // FileAttributes
        put_u32_le(&mut body[32..36], share_access); // ShareAccess
        put_u32_le(&mut body[36..40], disposition); // CreateDisposition
        put_u32_le(&mut body[40..44], options); // CreateOptions
        // NameOffset = 120 (header 64 + body 56)
        put_u16_le(&mut body[44..46], 120);
        put_u16_le(&mut body[46..48], path_u16.len() as u16); // NameLength
        put_u32_le(&mut body[48..52], 0); // CreateContextsOffset
        put_u32_le(&mut body[52..56], 0); // CreateContextsLength
        body.extend_from_slice(&path_u16);

        // Pad to even if needed (path_u16 should already be even)
        if body.len() % 2 != 0 {
            body.push(0);
        }

        let hdr = self.build_header(SMB2_CREATE);
        let resp = self.transact(&hdr, &body)?;
        let status = Self::response_status(&resp);

        if status != STATUS_SUCCESS {
            // Map to VFS errors
            return match status {
                0xC0000034 => Err(FsError::NotFound),       // STATUS_OBJECT_NAME_NOT_FOUND
                0xC000003A => Err(FsError::NotFound),       // STATUS_OBJECT_PATH_NOT_FOUND
                0xC0000035 => Err(FsError::AlreadyExists),  // STATUS_OBJECT_NAME_COLLISION
                0xC0000022 => Err(FsError::PermissionDenied), // STATUS_ACCESS_DENIED
                _ => {
                    crate::serial_println!("[SMBFS] Create failed: 0x{:08X} path='{}'", status, path);
                    Err(FsError::IoError)
                }
            };
        }

        // Parse response: body starts at offset 64
        let rbody = &resp[SMB2_HEADER_SIZE..];
        if rbody.len() < 88 {
            return Err(FsError::IoError);
        }

        // FileId at offset 64 from body start
        let mut file_id = [0u8; 16];
        file_id.copy_from_slice(&rbody[64..80]);

        // FileAttributes at offset 56
        let attrs = get_u32_le(&rbody[56..60]);

        // EndOfFile at offset 48
        let end_of_file = get_u64_le(&rbody[48..56]);

        Ok((file_id, end_of_file, attrs))
    }

    /// SMB2 CLOSE — close a file handle.
    fn smb2_close(&mut self, file_id: &[u8; 16]) -> Result<(), FsError> {
        let mut body = vec![0u8; 24];
        put_u16_le(&mut body[0..2], 24); // StructureSize
        put_u16_le(&mut body[2..4], 0);  // Flags
        put_u32_le(&mut body[4..8], 0);  // Reserved
        body[8..24].copy_from_slice(file_id);

        let hdr = self.build_header(SMB2_CLOSE);
        let _resp = self.transact(&hdr, &body)?;
        // We don't check status on close — best effort
        Ok(())
    }

    /// SMB2 READ — read data from a file.
    fn smb2_read(&mut self, file_id: &[u8; 16], offset: u64, length: u32) -> Result<Vec<u8>, FsError> {
        let read_len = length.min(self.max_read_size);

        let mut body = vec![0u8; 48]; // Fixed size before file_id
        put_u16_le(&mut body[0..2], 49); // StructureSize
        body[2] = 0; // Padding
        body[3] = 0; // Flags
        put_u32_le(&mut body[4..8], read_len); // Length
        put_u64_le(&mut body[8..16], offset); // Offset
        body[16..32].copy_from_slice(file_id); // FileId
        put_u32_le(&mut body[32..36], 0); // MinimumCount
        put_u32_le(&mut body[36..40], 0); // Channel
        put_u32_le(&mut body[40..44], 0); // RemainingBytes
        put_u16_le(&mut body[44..46], 0); // ReadChannelInfoOffset
        put_u16_le(&mut body[46..48], 0); // ReadChannelInfoLength
        // Add 1 byte of padding (StructureSize is 49 = odd, so body needs a buffer byte)
        body.push(0);

        let hdr = self.build_header(SMB2_READ);
        let resp = self.transact(&hdr, &body)?;
        let status = Self::response_status(&resp);

        if status != STATUS_SUCCESS {
            if status == 0xC0000011 { // STATUS_END_OF_FILE
                return Ok(Vec::new());
            }
            crate::serial_println!("[SMBFS] Read failed: 0x{:08X}", status);
            return Err(FsError::IoError);
        }

        // Parse read response
        let rbody = &resp[SMB2_HEADER_SIZE..];
        if rbody.len() < 16 {
            return Err(FsError::IoError);
        }
        let data_offset = rbody[2] as usize; // DataOffset (relative to beginning of header)
        let data_len = get_u32_le(&rbody[4..8]) as usize;

        // DataOffset is relative to start of SMB2 header
        if data_offset < SMB2_HEADER_SIZE || data_offset + data_len > resp.len() {
            // Fallback: data might be right after the response body header
            if rbody.len() >= 16 + data_len {
                return Ok(Vec::from(&rbody[16..16 + data_len]));
            }
            return Err(FsError::IoError);
        }

        Ok(Vec::from(&resp[data_offset..data_offset + data_len]))
    }

    /// SMB2 WRITE — write data to a file.
    fn smb2_write(&mut self, file_id: &[u8; 16], offset: u64, data: &[u8]) -> Result<u32, FsError> {
        let write_len = data.len().min(self.max_write_size as usize);

        let mut body = vec![0u8; 48];
        put_u16_le(&mut body[0..2], 49); // StructureSize
        // DataOffset = 112 (header 64 + body 48)
        put_u16_le(&mut body[2..4], 112);
        put_u32_le(&mut body[4..8], write_len as u32); // Length
        put_u64_le(&mut body[8..16], offset); // Offset
        body[16..32].copy_from_slice(file_id); // FileId
        put_u32_le(&mut body[32..36], 0); // Channel
        put_u32_le(&mut body[36..40], 0); // RemainingBytes
        put_u16_le(&mut body[40..42], 0); // WriteChannelInfoOffset
        put_u16_le(&mut body[42..44], 0); // WriteChannelInfoLength
        put_u32_le(&mut body[44..48], 0); // Flags
        body.extend_from_slice(&data[..write_len]);

        let hdr = self.build_header(SMB2_WRITE);
        let resp = self.transact(&hdr, &body)?;
        let status = Self::response_status(&resp);

        if status != STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] Write failed: 0x{:08X}", status);
            return Err(FsError::IoError);
        }

        // Parse response: BytesWritten at offset 4 in body
        let rbody = &resp[SMB2_HEADER_SIZE..];
        if rbody.len() >= 8 {
            Ok(get_u32_le(&rbody[4..8]))
        } else {
            Ok(write_len as u32)
        }
    }

    /// SMB2 QUERY_DIRECTORY — list directory contents.
    fn smb2_query_directory(&mut self, file_id: &[u8; 16]) -> Result<Vec<DirEntry>, FsError> {
        let mut entries = Vec::new();
        let pattern = to_utf16le("*");
        let mut first = true;

        loop {
            let mut body = vec![0u8; 32];
            put_u16_le(&mut body[0..2], 33); // StructureSize
            body[2] = FILE_BOTH_DIR_INFORMATION; // FileInformationClass
            body[3] = if first { 0x01 } else { 0x00 }; // Flags: SMB2_RESTART_SCANS on first
            put_u32_le(&mut body[4..8], 0); // FileIndex
            body[8..24].copy_from_slice(file_id); // FileId
            // FileNameOffset = 96 (header 64 + body 32)
            put_u16_le(&mut body[24..26], 96);
            put_u16_le(&mut body[26..28], if first { pattern.len() as u16 } else { 0 }); // FileNameLength
            put_u32_le(&mut body[28..32], 65536); // OutputBufferLength
            if first {
                body.push(0); // padding byte (StructureSize=33)
                body.extend_from_slice(&pattern);
            } else {
                body.push(0); // padding byte
            }

            let hdr = self.build_header(SMB2_QUERY_DIRECTORY);
            let resp = self.transact(&hdr, &body)?;
            let status = Self::response_status(&resp);

            if status == STATUS_NO_MORE_FILES {
                break;
            }
            if status != STATUS_SUCCESS {
                crate::serial_println!("[SMBFS] QueryDirectory failed: 0x{:08X}", status);
                return Err(FsError::IoError);
            }

            // Parse response
            let rbody = &resp[SMB2_HEADER_SIZE..];
            if rbody.len() < 8 {
                break;
            }
            let out_offset = get_u16_le(&rbody[2..4]) as usize;
            let out_length = get_u32_le(&rbody[4..8]) as usize;

            if out_offset < SMB2_HEADER_SIZE || out_offset + out_length > resp.len() {
                break;
            }

            let dir_data = &resp[out_offset..out_offset + out_length];
            self.parse_dir_entries(dir_data, &mut entries);

            first = false;
        }

        Ok(entries)
    }

    /// Parse FILE_BOTH_DIR_INFORMATION entries from a QueryDirectory response buffer.
    fn parse_dir_entries(&self, data: &[u8], entries: &mut Vec<DirEntry>) {
        let mut offset = 0usize;

        loop {
            if offset + 94 > data.len() {
                break;
            }

            let next_offset = get_u32_le(&data[offset..offset + 4]) as usize;
            let file_name_length = get_u32_le(&data[offset + 60..offset + 64]) as usize;
            let end_of_file = get_u64_le(&data[offset + 40..offset + 48]);
            let file_attributes = get_u32_le(&data[offset + 56..offset + 60]);

            // File name starts at offset 94 (after ShortName[24])
            let name_start = offset + 94;
            if name_start + file_name_length > data.len() {
                break;
            }

            let name_bytes = &data[name_start..name_start + file_name_length];
            let name = from_utf16le(name_bytes);

            // Skip . and ..
            if name != "." && name != ".." {
                let file_type = if file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    FileType::Directory
                } else {
                    FileType::Regular
                };

                entries.push(DirEntry {
                    name,
                    file_type,
                    size: end_of_file as u32,
                    is_symlink: false,
                    uid: 0,
                    gid: 0,
                    mode: 0o755,
                });
            }

            if next_offset == 0 {
                break;
            }
            offset += next_offset;
        }
    }

    /// SMB2 SET_INFO — used for delete-on-close.
    fn smb2_set_delete_on_close(&mut self, file_id: &[u8; 16]) -> Result<(), FsError> {
        let mut body = vec![0u8; 32];
        put_u16_le(&mut body[0..2], 33); // StructureSize
        body[2] = 0x01; // InfoType = FILE
        body[3] = 0x0D; // FileInfoClass = FileDispositionInformation
        put_u32_le(&mut body[4..8], 1); // BufferLength
        // BufferOffset = 96 (header 64 + body 32)
        put_u16_le(&mut body[8..10], 96);
        put_u16_le(&mut body[10..12], 0); // Reserved
        put_u32_le(&mut body[12..16], 0); // AdditionalInformation
        body[16..32].copy_from_slice(file_id); // FileId
        // Disposition info: DeletePending = 1
        body.push(1);

        let hdr = self.build_header(SMB2_SET_INFO);
        let resp = self.transact(&hdr, &body)?;
        let status = Self::response_status(&resp);
        if status != STATUS_SUCCESS {
            crate::serial_println!("[SMBFS] SetInfo delete failed: 0x{:08X}", status);
            return Err(FsError::IoError);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Inode / Path Mapping
    // -----------------------------------------------------------------------

    /// Get or register a path in the inode map. Returns a stable inode.
    fn path_to_inode(&mut self, path: &str) -> u32 {
        let clean = normalize_smb_path(path);
        let hash = hash_path(&clean);
        // Check if already registered
        if !self.path_map.iter().any(|h| h.inode == hash) {
            self.path_map.push(SmbHandle {
                inode: hash,
                path: clean,
            });
        }
        hash
    }

    /// Look up a path from an inode.
    fn inode_to_path(&self, inode: u32) -> Option<&str> {
        self.path_map.iter()
            .find(|h| h.inode == inode)
            .map(|h| h.path.as_str())
    }

    // -----------------------------------------------------------------------
    // Filesystem trait methods (called from VFS)
    // -----------------------------------------------------------------------

    /// Look up a path and return (inode, file_type, size).
    pub fn lookup(&mut self, path: &str) -> Result<(u32, FileType, u32), FsError> {
        let clean = normalize_smb_path(path);

        // Root directory
        if clean.is_empty() || clean == "/" {
            let inode = self.path_to_inode("/");
            return Ok((inode, FileType::Directory, 0));
        }

        // Open file to get attributes, then immediately close
        let (file_id, end_of_file, attrs) = self.smb2_create(
            &clean,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            0, // Let server decide if directory or file
        )?;
        let _ = self.smb2_close(&file_id);

        let file_type = if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            FileType::Directory
        } else {
            FileType::Regular
        };

        let inode = self.path_to_inode(&clean);
        Ok((inode, file_type, end_of_file as u32))
    }

    /// Read bytes from a file at the given offset.
    pub fn read_file(&mut self, inode: u32, offset: u32, buf: &mut [u8]) -> Result<usize, FsError> {
        let path = String::from(self.inode_to_path(inode)
            .ok_or(FsError::NotFound)?);

        let (file_id, _eof, _attrs) = self.smb2_create(
            &path,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
        )?;

        let data = self.smb2_read(&file_id, offset as u64, buf.len() as u32)?;
        let _ = self.smb2_close(&file_id);

        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    /// Write bytes to a file at the given offset.
    pub fn write_file(&mut self, inode: u32, offset: u32, data: &[u8]) -> Result<usize, FsError> {
        let path = String::from(self.inode_to_path(inode)
            .ok_or(FsError::NotFound)?);

        let (file_id, _eof, _attrs) = self.smb2_create(
            &path,
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
        )?;

        let written = self.smb2_write(&file_id, offset as u64, data)?;
        let _ = self.smb2_close(&file_id);

        Ok(written as usize)
    }

    /// Read directory entries.
    pub fn read_dir(&mut self, inode: u32) -> Result<Vec<DirEntry>, FsError> {
        let path = String::from(self.inode_to_path(inode)
            .ok_or(FsError::NotFound)?);

        let (file_id, _eof, _attrs) = self.smb2_create(
            &path,
            FILE_LIST_DIRECTORY | GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
        )?;

        let entries = self.smb2_query_directory(&file_id)?;
        let _ = self.smb2_close(&file_id);

        Ok(entries)
    }

    /// Create a new file or directory.
    pub fn create_entry(&mut self, parent_inode: u32, name: &str, file_type: FileType) -> Result<u32, FsError> {
        let parent_path = String::from(self.inode_to_path(parent_inode)
            .ok_or(FsError::NotFound)?);

        let full_path = if parent_path == "/" || parent_path.is_empty() {
            let mut p = String::from("/");
            p.push_str(name);
            p
        } else {
            let mut p = parent_path;
            p.push('/');
            p.push_str(name);
            p
        };

        let options = if file_type == FileType::Directory {
            FILE_DIRECTORY_FILE
        } else {
            FILE_NON_DIRECTORY_FILE
        };

        let (file_id, _eof, _attrs) = self.smb2_create(
            &full_path,
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_CREATE,
            options,
        )?;
        let _ = self.smb2_close(&file_id);

        let inode = self.path_to_inode(&full_path);
        Ok(inode)
    }

    /// Delete a file or directory by name under a parent.
    pub fn delete_entry(&mut self, parent_inode: u32, name: &str) -> Result<(), FsError> {
        let parent_path = String::from(self.inode_to_path(parent_inode)
            .ok_or(FsError::NotFound)?);

        let full_path = if parent_path == "/" || parent_path.is_empty() {
            let mut p = String::from("/");
            p.push_str(name);
            p
        } else {
            let mut p = parent_path;
            p.push('/');
            p.push_str(name);
            p
        };

        // Open with DELETE access and FILE_DELETE_ON_CLOSE
        let (file_id, _eof, _attrs) = self.smb2_create(
            &full_path,
            DELETE | GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_DELETE_ON_CLOSE,
        )?;
        let _ = self.smb2_close(&file_id);

        // Remove from path map
        self.path_map.retain(|h| {
            let hp = h.path.as_str();
            hp != full_path && !hp.starts_with(&{
                let mut prefix = full_path.clone();
                prefix.push('/');
                prefix
            })
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Utility Functions
// ---------------------------------------------------------------------------

/// Parse a decimal string into u32.
fn parse_u32(s: &str) -> Option<u32> {
    let mut val: u32 = 0;
    for &b in s.as_bytes() {
        if b < b'0' || b > b'9' {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

/// Parse an IPv4 address string like "192.168.1.1" into 4 bytes.
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    for part in s.split('.') {
        if idx >= 4 {
            return None;
        }
        let val = parse_u32(part)?;
        if val > 255 {
            return None;
        }
        parts[idx] = val as u8;
        idx += 1;
    }
    if idx == 4 { Some(parts) } else { None }
}

/// Convert a UTF-8 string to UTF-16LE bytes.
fn to_utf16le(s: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    for c in s.chars() {
        let v = c as u32;
        if v <= 0xFFFF {
            buf.push(v as u8);
            buf.push((v >> 8) as u8);
        } else {
            // Surrogate pair for supplementary characters
            let v = v - 0x10000;
            let hi = (v >> 10) + 0xD800;
            let lo = (v & 0x3FF) + 0xDC00;
            buf.push(hi as u8);
            buf.push((hi >> 8) as u8);
            buf.push(lo as u8);
            buf.push((lo >> 8) as u8);
        }
    }
    buf
}

/// Convert UTF-16LE bytes to a String.
fn from_utf16le(data: &[u8]) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let code = (data[i] as u16) | ((data[i + 1] as u16) << 8);
        if code == 0 {
            break;
        }
        if let Some(c) = char::from_u32(code as u32) {
            s.push(c);
        }
        i += 2;
    }
    s
}

/// Normalize an SMB path: strip leading slashes, replace double slashes.
fn normalize_smb_path(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return String::from("/");
    }
    let mut result = String::from("/");
    let mut last_was_slash = false;
    for c in trimmed.chars() {
        if c == '/' || c == '\\' {
            if !last_was_slash {
                result.push('/');
                last_was_slash = true;
            }
        } else {
            result.push(c);
            last_was_slash = false;
        }
    }
    // Remove trailing slash (unless root)
    if result.len() > 1 && result.ends_with('/') {
        result.pop();
    }
    result
}

/// Simple hash function for path→inode mapping.
fn hash_path(path: &str) -> u32 {
    let mut hash: u32 = 5381;
    for &b in path.as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    // Ensure non-zero (0 has special meaning in some FS code)
    if hash == 0 { 1 } else { hash }
}

/// Build NTLMSSP Negotiate message (Type 1) for anonymous authentication.
fn build_ntlmssp_negotiate() -> Vec<u8> {
    let mut msg = Vec::new();
    // Signature
    msg.extend_from_slice(NTLMSSP_SIGNATURE);
    // MessageType = NEGOTIATE (1)
    msg.extend_from_slice(&NTLMSSP_NEGOTIATE.to_le_bytes());
    // NegotiateFlags
    let flags = NTLMSSP_NEGOTIATE_UNICODE | NTLMSSP_NEGOTIATE_NTLM;
    msg.extend_from_slice(&flags.to_le_bytes());
    // DomainNameFields: Len(2) + MaxLen(2) + Offset(4) = 0
    msg.extend_from_slice(&[0u8; 8]);
    // WorkstationFields: Len(2) + MaxLen(2) + Offset(4) = 0
    msg.extend_from_slice(&[0u8; 8]);
    msg
}

/// Build NTLMSSP Auth message (Type 3) with empty credentials for guest/anonymous.
fn build_ntlmssp_auth() -> Vec<u8> {
    let mut msg = Vec::new();
    // Signature
    msg.extend_from_slice(NTLMSSP_SIGNATURE);
    // MessageType = AUTH (3)
    msg.extend_from_slice(&NTLMSSP_AUTH.to_le_bytes());
    // All security buffer fields point to offset 88 with length 0 (anonymous)
    let offset: u32 = 88;
    // LmChallengeResponse: Len=0, MaxLen=0, Offset
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&offset.to_le_bytes());
    // NtChallengeResponse: Len=0, MaxLen=0, Offset
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&offset.to_le_bytes());
    // DomainName: Len=0, MaxLen=0, Offset
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&offset.to_le_bytes());
    // UserName: Len=0, MaxLen=0, Offset
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&offset.to_le_bytes());
    // Workstation: Len=0, MaxLen=0, Offset
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&offset.to_le_bytes());
    // EncryptedRandomSession: Len=0, MaxLen=0, Offset
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&0u16.to_le_bytes());
    msg.extend_from_slice(&offset.to_le_bytes());
    // NegotiateFlags
    let flags = NTLMSSP_NEGOTIATE_UNICODE | NTLMSSP_NEGOTIATE_NTLM;
    msg.extend_from_slice(&flags.to_le_bytes());

    // Pad to offset 88 if needed
    while msg.len() < offset as usize {
        msg.push(0);
    }

    msg
}

// ---------------------------------------------------------------------------
// Byte helpers (little-endian)
// ---------------------------------------------------------------------------

fn put_u16_le(buf: &mut [u8], val: u16) {
    buf[0] = val as u8;
    buf[1] = (val >> 8) as u8;
}

fn put_u32_le(buf: &mut [u8], val: u32) {
    buf[0] = val as u8;
    buf[1] = (val >> 8) as u8;
    buf[2] = (val >> 16) as u8;
    buf[3] = (val >> 24) as u8;
}

fn put_u64_le(buf: &mut [u8], val: u64) {
    for i in 0..8 {
        buf[i] = (val >> (i * 8)) as u8;
    }
}

fn get_u16_le(buf: &[u8]) -> u16 {
    (buf[0] as u16) | ((buf[1] as u16) << 8)
}

fn get_u32_le(buf: &[u8]) -> u32 {
    (buf[0] as u32) | ((buf[1] as u32) << 8) | ((buf[2] as u32) << 16) | ((buf[3] as u32) << 24)
}

fn get_u64_le(buf: &[u8]) -> u64 {
    let mut val: u64 = 0;
    for i in 0..8 {
        val |= (buf[i] as u64) << (i * 8);
    }
    val
}
