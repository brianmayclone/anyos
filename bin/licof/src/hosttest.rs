use crate::config::LicoConfig;
use crate::model::PackageInfo;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct DownloadReport {
    pub index_gz_bytes: usize,
    pub index_bytes: usize,
    pub packages: usize,
    pub downloaded_bytes: u64,
    pub cache_dir: PathBuf,
}

pub fn run_full_download_test(tmp_root: &Path) -> Result<DownloadReport, String> {
    let cache_dir = tmp_root.join("cache");
    let db_dir = tmp_root.join("db");
    let rootfs_dir = tmp_root.join("rootfs");
    fs::create_dir_all(&cache_dir).map_err(|err| format!("create cache dir: {err}"))?;
    fs::create_dir_all(&db_dir).map_err(|err| format!("create db dir: {err}"))?;
    fs::create_dir_all(&rootfs_dir).map_err(|err| format!("create rootfs dir: {err}"))?;

    let mut config = LicoConfig::defaults();
    config.root = tmp_root.to_string_lossy().into_owned();
    config.cache = cache_dir.to_string_lossy().into_owned();
    config.db = db_dir.to_string_lossy().into_owned();
    config.installed_db = db_dir.join("installed").to_string_lossy().into_owned();
    config.rootfs_dir = rootfs_dir.to_string_lossy().into_owned();
    config.default_rootfs = rootfs_dir.join("debian").to_string_lossy().into_owned();

    let index_url = config.package_index_url();
    let index_gz = http_get(&index_url, 0)?;
    if index_gz.is_empty() {
        return Err(format!("empty package index from {index_url}"));
    }
    fs::write(config.package_index_gz(), &index_gz)
        .map_err(|err| format!("write package index gzip: {err}"))?;

    let index = libzip_client::gunzip(&index_gz)
        .ok_or_else(|| "gunzip package index failed".to_string())?;
    if index.len() < 20_000_000 || !index.starts_with(b"Package: ") {
        return Err(format!(
            "unexpected package index after gunzip: {} bytes",
            index.len()
        ));
    }
    fs::write(config.package_index_txt(), &index)
        .map_err(|err| format!("write package index text: {err}"))?;

    for pkg in &config.index_required_packages {
        find_package_in_bytes(&config, &index, pkg)
            .ok_or_else(|| format!("required package '{pkg}' not found in index"))?;
    }

    let mut resolved = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for pkg in &config.bootstrap_seed {
        resolve_package(&config, &index, pkg, &mut resolved, &mut visiting, 0)?;
    }

    let mut downloaded_bytes = 0u64;
    for package in &resolved {
        let info = find_package_in_bytes(&config, &index, package)
            .ok_or_else(|| format!("resolved package '{package}' disappeared from index"))?;
        let url = format!("{}/{}", config.apt_base, info.filename);
        let data = http_get(&url, 0)?;
        verify_deb(&info, &data)?;
        verify_deb_payload(&info, &data)?;
        downloaded_bytes += data.len() as u64;
        let path = cache_dir.join(cache_deb_name(&info));
        fs::write(&path, &data).map_err(|err| format!("write {}: {err}", path.display()))?;
    }

    Ok(DownloadReport {
        index_gz_bytes: index_gz.len(),
        index_bytes: index.len(),
        packages: resolved.len(),
        downloaded_bytes,
        cache_dir,
    })
}

fn resolve_package(
    config: &LicoConfig,
    index: &[u8],
    wanted: &str,
    resolved: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
    depth: u8,
) -> Result<(), String> {
    if depth > 64 {
        return Err(format!("dependency recursion too deep at '{wanted}'"));
    }
    let wanted = preferred_package(wanted).unwrap_or(wanted);
    let info = find_package_in_bytes(config, index, wanted)
        .ok_or_else(|| format!("package '{wanted}' not found in index"))?;
    if resolved.contains(&info.package) {
        return Ok(());
    }
    if !visiting.insert(info.package.clone()) {
        return Ok(());
    }

    for group in parse_depends(&info.pre_depends) {
        resolve_dependency_group(config, index, &group, resolved, visiting, depth + 1)?;
    }
    for group in parse_depends(&info.depends) {
        resolve_dependency_group(config, index, &group, resolved, visiting, depth + 1)?;
    }

    visiting.remove(&info.package);
    resolved.insert(info.package);
    Ok(())
}

fn resolve_dependency_group(
    config: &LicoConfig,
    index: &[u8],
    alternatives: &[String],
    resolved: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
    depth: u8,
) -> Result<(), String> {
    for dep in alternatives {
        let dep = preferred_package(dep).unwrap_or(dep);
        if find_package_in_bytes(config, index, dep).is_none() {
            continue;
        }
        return resolve_package(config, index, dep, resolved, visiting, depth);
    }
    if alternatives.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "no dependency alternative found for '{}'",
            alternatives.join(" | ")
        ))
    }
}

fn find_package_in_bytes(config: &LicoConfig, index: &[u8], wanted: &str) -> Option<PackageInfo> {
    let mut start = 0usize;
    let mut pos = 0usize;
    let mut newline_run = 0usize;

    while pos < index.len() {
        let b = index[pos];
        if b == b'\n' {
            newline_run += 1;
            if newline_run >= 2 {
                if let Some(info) = package_info_from_para(config, &index[start..=pos], wanted) {
                    return Some(info);
                }
                start = pos + 1;
                newline_run = 0;
            }
        } else if b != b'\r' {
            newline_run = 0;
        }
        pos += 1;
    }

    if start < index.len() {
        package_info_from_para(config, &index[start..], wanted)
    } else {
        None
    }
}

fn package_info_from_para(config: &LicoConfig, para: &[u8], wanted: &str) -> Option<PackageInfo> {
    let package = field_value(para, b"Package")?;
    let exact = package == wanted;
    if !exact && !provides_package_bytes(para, wanted) {
        return None;
    }
    let arch = field_value(para, b"Architecture").unwrap_or_default();
    if arch != config.apt_arch && arch != "all" {
        return None;
    }
    Some(PackageInfo {
        package,
        version: field_value(para, b"Version").unwrap_or_else(|| String::from("unknown")),
        filename: field_value(para, b"Filename")?,
        size: field_value(para, b"Size")
            .and_then(|s| parse_decimal(&s))
            .unwrap_or(0),
        md5: field_value(para, b"MD5sum").unwrap_or_default(),
        depends: field_value(para, b"Depends").unwrap_or_default(),
        pre_depends: field_value(para, b"Pre-Depends").unwrap_or_default(),
    })
}

fn field_value(para: &[u8], key: &[u8]) -> Option<String> {
    let mut line_start = 0usize;
    let mut collecting = false;
    let mut out = String::new();
    while line_start < para.len() {
        let mut line_end = line_start;
        while line_end < para.len() && para[line_end] != b'\n' {
            line_end += 1;
        }
        let mut line = &para[line_start..line_end];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if line.len() > key.len() + 2
            && &line[..key.len()] == key
            && line[key.len()] == b':'
            && line[key.len() + 1] == b' '
        {
            out.clear();
            push_utf8_trimmed(&mut out, &line[key.len() + 2..]);
            collecting = true;
        } else if collecting && (line.first() == Some(&b' ') || line.first() == Some(&b'\t')) {
            if !out.is_empty() {
                out.push(' ');
            }
            push_utf8_trimmed(&mut out, line);
        } else if collecting {
            break;
        }
        line_start = line_end.saturating_add(1);
    }
    if collecting {
        Some(out)
    } else {
        None
    }
}

fn push_utf8_trimmed(out: &mut String, bytes: &[u8]) {
    if let Ok(s) = core::str::from_utf8(bytes) {
        out.push_str(s.trim());
    }
}

fn provides_package_bytes(para: &[u8], wanted: &str) -> bool {
    let provides = field_value(para, b"Provides").unwrap_or_default();
    for provided in parse_depends(&provides) {
        if provided.iter().any(|name| name == wanted) {
            return true;
        }
    }
    false
}

fn preferred_package(wanted: &str) -> Option<&'static str> {
    match wanted {
        "awk" => Some("mawk"),
        _ => None,
    }
}

fn parse_depends(raw: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let mut group = Vec::new();
        for alt in part.split('|') {
            let Some(name) = dependency_name(alt.trim()) else {
                continue;
            };
            if !group.iter().any(|p| p == &name) {
                group.push(name);
            }
        }
        if !group.is_empty() {
            out.push(group);
        }
    }
    out
}

fn dependency_name(raw: &str) -> Option<String> {
    let mut name = raw.split_ascii_whitespace().next().unwrap_or("").trim();
    if let Some(pos) = name.find(':') {
        name = &name[..pos];
    }
    if name.is_empty() {
        None
    } else {
        Some(String::from(name))
    }
}

fn verify_deb(info: &PackageInfo, data: &[u8]) -> Result<(), String> {
    if info.size != 0 && data.len() != info.size {
        return Err(format!(
            "{} size mismatch: expected {}, got {}",
            info.package,
            info.size,
            data.len()
        ));
    }
    if !info.md5.is_empty() {
        let actual = anyos_std::crypto::md5_hex(data);
        let actual = core::str::from_utf8(&actual).unwrap_or("");
        if actual != info.md5 {
            return Err(format!(
                "{} md5 mismatch: expected {}, got {}",
                info.package, info.md5, actual
            ));
        }
    }
    Ok(())
}

fn verify_deb_payload(info: &PackageInfo, data: &[u8]) -> Result<(), String> {
    let reader = if let Some(tar_data) = ar_entry(data, "data.tar.gz") {
        libzip_client::TarReader::from_bytes(&tar_data)
    } else if let Some(xz_data) = ar_entry(data, "data.tar.xz") {
        let tar_data = libzip_client::unxz(&xz_data)
            .ok_or_else(|| format!("{} data.tar.xz decompression failed", info.package))?;
        libzip_client::TarReader::from_bytes(&tar_data)
    } else {
        return Err(format!(
            "{} has no supported data.tar.* member",
            info.package
        ));
    };
    let reader = reader.ok_or_else(|| format!("{} data tar parse failed", info.package))?;
    if reader.entry_count() == 0 {
        return Err(format!("{} data tar is empty", info.package));
    }
    Ok(())
}

fn ar_entry(data: &[u8], wanted: &str) -> Option<Vec<u8>> {
    if data.len() < 8 || &data[..8] != b"!<arch>\n" {
        return None;
    }
    let mut pos = 8usize;
    while pos + 60 <= data.len() {
        let header = &data[pos..pos + 60];
        let raw_name = core::str::from_utf8(&header[..16]).ok()?.trim();
        let name = raw_name.trim_end_matches('/');
        let size_str = core::str::from_utf8(&header[48..58]).ok()?.trim();
        let size = parse_decimal(size_str)?;
        let start = pos + 60;
        let end = start.checked_add(size)?;
        if end > data.len() {
            return None;
        }
        if name == wanted {
            return Some(data[start..end].to_vec());
        }
        pos = end + (size & 1);
    }
    None
}

fn parse_decimal(s: &str) -> Option<usize> {
    let mut out = 0usize;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        out = out.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(out)
}

fn cache_deb_name(info: &PackageInfo) -> String {
    let basename = info.filename.rsplit('/').next().unwrap_or(&info.filename);
    let mut out = String::new();
    push_cache_safe(&mut out, &info.package);
    out.push('_');
    push_cache_safe(&mut out, &info.version);
    out.push('_');
    push_cache_safe(&mut out, basename);
    out
}

fn push_cache_safe(out: &mut String, value: &str) {
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
}

fn http_get(url: &str, redirects: u8) -> Result<Vec<u8>, String> {
    if redirects > 5 {
        return Err(format!("too many redirects for {url}"));
    }
    let parts = parse_http_url(url)?;
    let mut stream = TcpStream::connect((parts.host.as_str(), parts.port))
        .map_err(|err| format!("connect {}:{}: {err}", parts.host, parts.port))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: licof-hosttest/1\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        parts.path, parts.host
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("send request {url}: {err}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| format!("read response {url}: {err}"))?;

    let header_end =
        find_header_end(&response).ok_or_else(|| format!("bad HTTP response {url}"))?;
    let headers = &response[..header_end];
    let mut body = response[header_end + 4..].to_vec();
    let status = parse_status(headers).ok_or_else(|| format!("missing HTTP status {url}"))?;
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        let location = header_value(headers, "Location")
            .ok_or_else(|| format!("redirect without Location from {url}"))?;
        let next = absolutize_url(url, &location)?;
        return http_get(&next, redirects + 1);
    }
    if status != 200 {
        return Err(format!("HTTP {status} for {url}"));
    }
    if header_value(headers, "Transfer-Encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        body = decode_chunked(&body)?;
    }
    Ok(body)
}

struct HttpUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_http_url(url: &str) -> Result<HttpUrl, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("only http:// URLs are supported by hosttest: {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("invalid HTTP port in {url}"))?;
            (host, port)
        }
        None => (authority, 80),
    };
    if host.is_empty() {
        return Err(format!("empty HTTP host in {url}"));
    }
    Ok(HttpUrl {
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_status(headers: &[u8]) -> Option<u16> {
    let line_end = headers.iter().position(|b| *b == b'\r')?;
    let line = core::str::from_utf8(&headers[..line_end]).ok()?;
    let mut parts = line.split_ascii_whitespace();
    let _http = parts.next()?;
    parts.next()?.parse().ok()
}

fn header_value(headers: &[u8], name: &str) -> Option<String> {
    let text = core::str::from_utf8(headers).ok()?;
    for line in text.split("\r\n").skip(1) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn absolutize_url(base: &str, location: &str) -> Result<String, String> {
    if location.starts_with("http://") {
        return Ok(location.to_string());
    }
    if location.starts_with("https://") {
        return Err(format!(
            "hosttest cannot follow https redirect from {base} to {location}"
        ));
    }
    if location.starts_with('/') {
        let parts = parse_http_url(base)?;
        return Ok(format!("http://{}{}", parts.host, location));
    }
    let prefix = base.rsplit_once('/').map(|(p, _)| p).unwrap_or(base);
    Ok(format!("{prefix}/{location}"))
}

fn decode_chunked(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut pos = 0usize;
    let mut out = Vec::new();
    loop {
        let line_end = find_crlf(data, pos).ok_or_else(|| "truncated chunk header".to_string())?;
        let size_line = core::str::from_utf8(&data[pos..line_end])
            .map_err(|_| "non-utf8 chunk header".to_string())?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("invalid chunk size '{size_hex}'"))?;
        pos = line_end + 2;
        if size == 0 {
            break;
        }
        let end = pos
            .checked_add(size)
            .ok_or_else(|| "chunk size overflow".to_string())?;
        if end + 2 > data.len() {
            return Err("truncated chunk body".to_string());
        }
        out.extend_from_slice(&data[pos..end]);
        pos = end + 2;
    }
    Ok(out)
}

fn find_crlf(data: &[u8], start: usize) -> Option<usize> {
    data[start..]
        .windows(2)
        .position(|w| w == b"\r\n")
        .map(|offset| start + offset)
}
