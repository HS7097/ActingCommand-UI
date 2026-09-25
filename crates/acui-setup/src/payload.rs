// SPDX-License-Identifier: GPL-3.0-only
//! The offline edition: `acsetup-full-<tag>.exe` is this program's own bytes
//! followed by one release — `SHA256SUMS` and every file it lists, as the
//! online path would fetch them — an index and a 125-byte trailer:
//!
//! ```text
//! acsetup.exe ‖ F1 ‖ … ‖ Fn ‖ INDEX ‖ ACSETUP-PAYLOAD-1 <start:%020d> <index length:%020d> <index sha256>\n
//! ```
//!
//! Which edition runs is read from the executable itself: bytes after the PE
//! image — up to the certificate table, less its at most seven bytes of NUL
//! padding, when the file is signed — must be a whole, consistent payload;
//! no such bytes is the online edition. A payload that is damaged, cut short
//! or inconsistent stops the wizard before it writes anything; it never falls
//! back to the network. Extracted, the release goes through the same checks as
//! a fetched one (`verify::run`).

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::fetch;
use crate::verify::{self, hex, Report, Step, Total};

const MAGIC: &[u8] = b"ACSETUP-PAYLOAD-1 ";
const TRAILER: u64 = 125;
const INDEX_LIMIT: u64 = 1 << 20;

/// One release file in the payload.
#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub sha256: String,
    pub offset: u64,
    pub len: u64,
}

/// The release this executable carries, checked at start.
pub struct Payload {
    file: File,
    pub tag: String,
    pub entries: Vec<Entry>,
    /// `MEMBERS.json` as carried, checked against the index and SHA256SUMS.
    pub members_text: String,
    pub members: (String, String),
}

/// The start of every message about a payload that cannot be used.
const DAMAGED: &str = "这个安装包损坏、被截断或不完整 / This installer is damaged, cut short or incomplete";
const REDOWNLOAD: &str = "请重新下载 acsetup-full-<tag>.exe（旁边的 .sha256 可核对），或改用在线版 acsetup.exe / Download acsetup-full-<tag>.exe again (check it against its .sha256), or use the online acsetup.exe";

fn damaged(detail: impl std::fmt::Display) -> String {
    format!("{DAMAGED}: {detail}\n{REDOWNLOAD}")
}

/// The offline edition's payload, `None` for the online edition. Any read or
/// parse failure is an error, never "online".
pub fn detect() -> Result<Option<Payload>, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("找不到本程序自身 / cannot locate this program: {error}"))?;
    let mut file = File::open(&exe)
        .map_err(|error| format!("无法读取本程序自身 / cannot read this program: {}: {error}", exe.display()))?;
    let length = file
        .metadata()
        .map_err(|error| format!("无法读取本程序自身 / cannot read this program: {}: {error}", exe.display()))?
        .len();
    let layout = pe_layout(&mut file, length).map_err(damaged)?;
    let end = effective_end(&mut file, &layout, length).map_err(damaged)?;
    match end.cmp(&layout.image_end) {
        std::cmp::Ordering::Equal => Ok(None),
        std::cmp::Ordering::Less => Err(damaged(format!(
            "PE 映像末端 {} 超出文件的有效末尾 {end} / the PE image ends at {} past the file's end {end}",
            layout.image_end, layout.image_end
        ))),
        std::cmp::Ordering::Greater => read_payload(file, layout.image_end, end).map(Some).map_err(damaged),
    }
}

struct Layout {
    image_end: u64,
    /// The certificate table's file offset, when the file is signed.
    certificates: Option<u64>,
}

fn read_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
    file.seek(SeekFrom::Start(offset))
        .and_then(|_| file.read_exact(buffer))
        .map_err(|error| format!("读取偏移 {offset} 处 {} 字节失败 / reading {} bytes at {offset} failed: {error}", buffer.len(), buffer.len()))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at.saturating_add(2))
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("PE 头在偏移 {at} 处不完整 / the PE header is incomplete at {at}"))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at.saturating_add(4))
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("PE 头在偏移 {at} 处不完整 / the PE header is incomplete at {at}"))
}

/// The end of the PE image — the furthest raw data of any section — and the
/// certificate table, from the DOS header, the PE signature, the COFF header,
/// the optional header's data directories and the section table.
fn pe_layout(file: &mut File, length: u64) -> Result<Layout, String> {
    const HEADERS: u64 = 64 * 1024;
    let size = length.min(HEADERS);
    let mut bytes = vec![0u8; usize::try_from(size).map_err(|error| error.to_string())?];
    read_at(file, 0, &mut bytes)?;
    if bytes.get(..2) != Some(b"MZ") {
        return Err("不是 PE 文件（缺 MZ）/ not a PE file (no MZ)".into());
    }
    let pe = usize::try_from(u32_at(&bytes, 0x3C)?).map_err(|error| error.to_string())?;
    if bytes.get(pe..pe.checked_add(4).ok_or("PE 偏移溢出 / PE offset overflow")?) != Some(b"PE\0\0") {
        return Err(format!("偏移 {pe} 处没有 PE 签名 / no PE signature at {pe}"));
    }
    let sections = usize::from(u16_at(&bytes, pe + 6)?);
    let optional_size = usize::from(u16_at(&bytes, pe + 20)?);
    let optional = pe + 24;
    let (count_at, directories) = match u16_at(&bytes, optional)? {
        0x10B => (optional + 92, optional + 96),
        0x20B => (optional + 108, optional + 112),
        magic => return Err(format!("可选头的 magic 无法识别 / unknown optional header magic: {magic:#x}")),
    };
    let certificates = match u32_at(&bytes, count_at)? > 4 && directories + 5 * 8 <= optional + optional_size {
        true => {
            let (offset, size) = (u32_at(&bytes, directories + 32)?, u32_at(&bytes, directories + 36)?);
            (size > 0).then_some(u64::from(offset))
        }
        false => None,
    };
    let table = optional
        .checked_add(optional_size)
        .ok_or("节表偏移溢出 / section table offset overflow")?;
    let mut image_end = 0u64;
    for section in 0..sections {
        let at = table + section * 40;
        let raw_size = u64::from(u32_at(&bytes, at + 16)?);
        let raw_pointer = u64::from(u32_at(&bytes, at + 20)?);
        if raw_size == 0 {
            continue;
        }
        let end = raw_pointer
            .checked_add(raw_size)
            .ok_or("节的末端溢出 / a section's end overflows")?;
        image_end = image_end.max(end);
    }
    if image_end == 0 || image_end > length {
        return Err(format!("PE 映像末端 {image_end} 不在文件内（长 {length}）/ the PE image end {image_end} is outside the file ({length} bytes)"));
    }
    Ok(Layout { image_end, certificates })
}

/// The file's effective end: the certificate table's offset less at most
/// seven bytes of NUL padding before it when the file is signed, else its
/// length.
fn effective_end(file: &mut File, layout: &Layout, length: u64) -> Result<u64, String> {
    let Some(certificates) = layout.certificates else {
        return Ok(length);
    };
    if certificates > length || certificates < layout.image_end {
        return Err(format!("证书表偏移 {certificates} 不在映像末端与文件末尾之间 / the certificate table offset {certificates} is not between the image end and the file end"));
    }
    let pad = (certificates - layout.image_end).min(7);
    let mut before = vec![0u8; pad as usize];
    read_at(file, certificates - pad, &mut before)?;
    let nuls = before.iter().rev().take_while(|byte| **byte == 0).count() as u64;
    Ok(certificates - nuls)
}

/// A decimal written the way the builder writes it: digits only, no sign.
fn digits(text: &[u8], what: &str) -> Result<u64, String> {
    if text.is_empty() || !text.iter().all(u8::is_ascii_digit) {
        return Err(format!("{what} 不是十进制数 / is not a decimal: {}", String::from_utf8_lossy(text)));
    }
    std::str::from_utf8(text)
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .ok_or_else(|| format!("{what} 超出范围 / is out of range: {}", String::from_utf8_lossy(text)))
}

/// A name the payload may carry: what the online path accepts, and not one
/// Windows would alter or the `.part` files could shadow.
fn name_ok(name: &str) -> bool {
    fetch::plain(name) && !name.starts_with('-') && !name.ends_with('.') && !name.ends_with(".part")
}

fn read_payload(mut file: File, start: u64, end: u64) -> Result<Payload, String> {
    if end - start < TRAILER {
        return Err(format!("映像之后只有 {} 字节，放不下尾部 / only {} bytes follow the image, too few for the trailer", end - start, end - start));
    }
    let mut trailer = [0u8; TRAILER as usize];
    read_at(&mut file, end - TRAILER, &mut trailer)?;
    if !trailer.starts_with(MAGIC) {
        return Err("映像之后的字节没有以载荷尾部结束（多半下载不完整）/ the bytes after the image do not end in a payload trailer (most likely an incomplete download)".into());
    }
    let fields = &trailer[MAGIC.len()..];
    // `%020d %020d <64 hex>\n`
    let shape_ok = fields.len() == 107
        && fields[20] == b' '
        && fields[41] == b' '
        && fields[106] == b'\n'
        && fields[42..106].iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !shape_ok {
        return Err(format!("尾部格式不对 / malformed trailer: {}", String::from_utf8_lossy(&trailer)));
    }
    let payload_start = digits(&fields[..20], "载荷起点 / payload start")?;
    let index_len = digits(&fields[21..41], "索引长度 / index length")?;
    let index_sha = String::from_utf8_lossy(&fields[42..106]).into_owned();
    if payload_start != start {
        return Err(format!("载荷起点应为映像末端 {start}，实为 {payload_start} / the payload should start at the image end {start}, not {payload_start}"));
    }
    if index_len > INDEX_LIMIT {
        return Err(format!("索引长 {index_len} 字节，超过上限 {INDEX_LIMIT} / the index is {index_len} bytes, over {INDEX_LIMIT}"));
    }
    let index_end = end - TRAILER;
    let index_at = index_end
        .checked_sub(index_len)
        .filter(|at| *at >= start)
        .ok_or_else(|| format!("索引长 {index_len} 超出载荷 / the index length {index_len} exceeds the payload"))?;
    let mut index = vec![0u8; index_len as usize];
    read_at(&mut file, index_at, &mut index)?;
    let actual = hex(&Sha256::digest(&index));
    if actual != index_sha {
        return Err(format!("索引的 sha256 应为 {index_sha}，实为 {actual} / the index sha256 should be {index_sha}, is {actual}"));
    }
    let index = String::from_utf8(index).map_err(|error| format!("索引不是 UTF-8 / the index is not UTF-8: {error}"))?;
    let Some(body) = index.strip_suffix('\n') else {
        return Err("索引没有以换行结尾 / the index does not end in a newline".into());
    };
    let mut lines = body.split('\n');
    let header: Vec<&str> = lines.next().unwrap_or_default().split(' ').collect();
    let tag = match header.as_slice() {
        ["acsetup-payload", "v1", tag] if fetch::plain(tag) => tag.to_string(),
        _ => return Err("索引头部应为「acsetup-payload v1 <tag>」/ the index header should be \"acsetup-payload v1 <tag>\"".into()),
    };
    let mut entries = Vec::new();
    let mut offset = start;
    let mut seen = std::collections::BTreeSet::new();
    for (row, line) in lines.enumerate() {
        let parts: Vec<&str> = line.split(' ').collect();
        let [sha, len, name] = parts.as_slice() else {
            return Err(format!("索引第 {} 行格式不对 / index line {} is malformed: {line}", row + 2, row + 2));
        };
        let canonical = !len.is_empty() && (*len == "0" || !len.starts_with('0'));
        let sha_ok = sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if !sha_ok || !canonical || !name_ok(name) {
            return Err(format!("索引第 {} 行格式不对 / index line {} is malformed: {line}", row + 2, row + 2));
        }
        let len = digits(len.as_bytes(), "文件长度 / file length")?;
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(format!("索引里的文件名重复 / a name repeats in the index: {name}"));
        }
        entries.push(Entry { name: name.to_string(), sha256: sha.to_string(), offset, len });
        offset = offset
            .checked_add(len)
            .ok_or("文件长度之和溢出 / the file lengths overflow")?;
    }
    if offset != index_at {
        return Err(format!(
            "载荷各文件之和应止于索引起点 {index_at}，实止于 {offset} / the files should end where the index starts ({index_at}), they end at {offset}"
        ));
    }
    if entries.first().map(|entry| entry.name.as_str()) != Some("SHA256SUMS") {
        return Err("索引的第一个文件应为 SHA256SUMS / the index's first file should be SHA256SUMS".into());
    }
    let mut payload = Payload { file, tag, entries, members_text: String::new(), members: Default::default() };
    // SHA256SUMS lists exactly the rest of the index, in its order, with the
    // same hashes; MEMBERS.json is among them.
    let sums = payload.read_whole("SHA256SUMS")?;
    let listed = verify::parse_sha256sums(&sums)?;
    let rest: Vec<(String, String)> = payload.entries[1..]
        .iter()
        .map(|entry| (entry.sha256.clone(), entry.name.clone()))
        .collect();
    if listed != rest {
        return Err("索引的文件与 SHA256SUMS 所列不一致（名、序或 sha256）/ the index and SHA256SUMS list different files (name, order or sha256)".into());
    }
    let members_text = payload.read_whole("MEMBERS.json")?;
    let members = verify::members_of(&members_text)?;
    if let Some((runtime, ui)) = tag_shas(&payload.tag) {
        if !members.0.starts_with(runtime) || !members.1.starts_with(ui) {
            return Err(format!(
                "标签 {} 与 MEMBERS.json 的提交不符 / the tag does not match MEMBERS.json's commits: runtime {} · ui {}",
                payload.tag, members.0, members.1
            ));
        }
    }
    payload.members_text = members_text;
    payload.members = members;
    Ok(payload)
}

/// `build-r<7 hex>-u<7 hex>`'s two short commits.
fn tag_shas(tag: &str) -> Option<(&str, &str)> {
    let rest = tag.strip_prefix("build-r")?;
    let (runtime, ui) = rest.split_once("-u")?;
    let short = |sha: &str| sha.len() == 7 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    (short(runtime) && short(ui)).then_some((runtime, ui))
}

impl Payload {
    /// A small file of the payload read whole, checked against the index.
    fn read_whole(&mut self, name: &str) -> Result<String, String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .cloned()
            .ok_or_else(|| format!("载荷里没有 {name} / the payload lacks {name}"))?;
        if entry.len > INDEX_LIMIT {
            return Err(format!("{name} 过大 / is too large: {} 字节 / bytes", entry.len));
        }
        let mut bytes = vec![0u8; entry.len as usize];
        read_at(&mut self.file, entry.offset, &mut bytes)?;
        let actual = hex(&Sha256::digest(&bytes));
        if actual != entry.sha256 {
            return Err(format!("{name} 的 sha256 应为 {}，实为 {actual} / {name} sha256 should be {}, is {actual}", entry.sha256, entry.sha256));
        }
        String::from_utf8(bytes).map_err(|error| format!("{name} 不是 UTF-8 / is not UTF-8: {error}"))
    }

    /// The same payload through a second handle on the same open file, for a
    /// worker thread.
    pub fn try_clone(&self) -> Result<Payload, String> {
        Ok(Payload {
            file: self.file.try_clone().map_err(|error| format!("无法复制文件句柄 / cannot clone the file handle: {error}"))?,
            tag: self.tag.clone(),
            entries: self.entries.clone(),
            members_text: self.members_text.clone(),
            members: self.members.clone(),
        })
    }

    pub fn total(&self) -> u64 {
        self.entries.iter().map(|entry| entry.len).sum()
    }

    /// Every file of the release written into `dir` through a `.part` file,
    /// its sha256 taken on the way and matched against the index before it is
    /// renamed; a rename a scanner briefly holds up is retried a few times.
    pub fn extract(&mut self, dir: &Path, report: Report<'_>) -> Result<(), String> {
        fs::create_dir_all(dir).map_err(|error| format!("无法创建 / cannot create {}: {error}", dir.display()))?;
        report.line(&format!("取出自带的发布件 {} / extracting the carried release into {}", self.tag, dir.display()))?;
        report.step(Step::Phase("取出自带的发布件 / Extracting the carried release", Some(Total::Bytes(self.total()))))?;
        let mut done = 0u64;
        for entry in self.entries.clone() {
            let target = dir.join(&entry.name);
            let part: PathBuf = dir.join(format!("{}.part", entry.name));
            let written = self.extract_one(&entry, &part, &mut done, report);
            let written = written.and_then(|()| rename_patiently(&part, &target));
            if let Err(reason) = written {
                return Err(fetch::discard(&part, reason));
            }
            report.line(&format!("已取出 / extracted: {}（{} 字节 / bytes）", entry.name, entry.len))?;
        }
        Ok(())
    }

    fn extract_one(&mut self, entry: &Entry, part: &Path, done: &mut u64, report: Report<'_>) -> Result<(), String> {
        let cannot = |error: std::io::Error| format!("无法写入 / cannot write {}: {error}", part.display());
        self.file
            .seek(SeekFrom::Start(entry.offset))
            .map_err(|error| format!("读取载荷失败 / reading the payload failed: {error}"))?;
        let mut out = File::create(part).map_err(cannot)?;
        let mut limited = (&mut self.file).take(entry.len);
        let (mut hasher, mut buffer, mut count) = (Sha256::new(), vec![0u8; 256 * 1024], 0u64);
        loop {
            let read = limited
                .read(&mut buffer)
                .map_err(|error| format!("读取载荷失败 / reading the payload failed: {error}"))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            out.write_all(&buffer[..read]).map_err(cannot)?;
            count += read as u64;
            *done += read as u64;
            report.step(Step::Done(*done))?;
        }
        out.sync_all().map_err(cannot)?;
        let actual = hex(&hasher.finalize());
        if count != entry.len || actual != entry.sha256 {
            return Err(format!(
                "{DAMAGED}: {} 应为 {} 字节 sha256 {}，实为 {count} 字节 sha256 {actual} / {} should be {} bytes sha256 {}, is {count} bytes sha256 {actual}\n{REDOWNLOAD}",
                entry.name, entry.len, entry.sha256, entry.name, entry.len, entry.sha256
            ));
        }
        Ok(())
    }
}

fn rename_patiently(part: &Path, target: &Path) -> Result<(), String> {
    let mut attempt = 0;
    loop {
        match fs::rename(part, target) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied && attempt < 5 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(200 * attempt));
            }
            Err(error) => return Err(format!("无法改名 / cannot rename {}: {error}", part.display())),
        }
    }
}
