// SPDX-License-Identifier: GPL-3.0-only
//! The graphics card for text recognition (DirectML), everything about it that computes: which of
//! the DXGI adapters this computer lists the options page offers and why the others are not, the
//! one it ticks first, the warning beside a card with little video memory of its own, the card a
//! plan names checked against this computer, and the reading of `check-config`'s
//! `vision.directml` once the install wrote the choice. The listing itself is [`crate::dxgi`]'s;
//! the configuration's `vision` section is [`crate::config::vision_section`].
//!
//! The rules mirror the Runtime's own resolution (Runtime `crates/vision-ffi/src/adapters.rs`,
//! design `rd-1031/perf-407/design-gpu1.md` §1.3), so that the wizard offers only an adapter the
//! Runtime resolves to:
//!
//! - an adapter is named by its stable identity `dxgi-pci:<vendor>:<device>:<subsys>:<rev>`
//!   (`DXGI_ADAPTER_DESC1`, lowercase hexadecimal of 4, 4, 8 and 2 digits), never by its DXGI
//!   index or its LUID, which change between boots;
//! - it is physical when the kernel's adapter type (`D3DKMT_ADAPTERTYPE`) was read, it renders,
//!   and it is neither a software device (by DXGI's flag or by the kernel's bit), an
//!   indirect-display device nor a paravirtualized one;
//! - of the adapters with the configured identity the Runtime keeps the physical ones: exactly
//!   one is chosen; two or more are narrowed to those whose PCI location the kernel gave, one of
//!   them is chosen, and otherwise the identity is `adapter_ambiguous`.
//!
//! What the options page lists: no software adapter (Microsoft's Basic Render Driver), and no
//! adapter that is not physical while a physical one has its identity (another listing of a card
//! already shown, which the Runtime passes over the same way). Every other adapter is listed: the
//! one the Runtime resolves its identity to can be chosen, each other is greyed with why not.
//! Integrated graphics are not left out: the person decides, warned beside a chosen card with
//! less than 2 GiB of video memory of its own.
//!
//! Whether the card really works only the first text recognition shows: `check-config` lists the
//! adapters, resolves the identity and hashes `DirectML.dll`, and creates no Direct3D 12 device
//! (Runtime `contracts/actingd-check-config.md`, "DirectML"). With `gpu_fallback` `cpu` a card
//! that fails then leaves the Runtime on the CPU with a `vision_gpu_unavailable` warning, which
//! the console's performance status shows.

use std::fmt;

use serde_json::Value;

use crate::child::outcome_text;
use crate::text::Said;

/// The prefix of an adapter's stable identity.
pub const IDENTITY_PREFIX: &str = "dxgi-pci:";
/// The hexadecimal widths of the identity's vendor, device, subsystem and revision.
const IDENTITY_WIDTHS: [usize; 4] = [4, 4, 8, 2];
/// `DXGI_ADAPTER_FLAG_SOFTWARE` of `DXGI_ADAPTER_DESC1::Flags`.
pub const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;
pub const GIB: u64 = 1 << 30;
/// Below this much video memory of its own a card looks like integrated graphics.
pub const INTEGRATED_BELOW: u64 = 2 * GIB;
/// The note (and the Runtime's warning) of a DirectML section that cannot run.
pub const UNAVAILABLE_CODE: &str = "vision_gpu_unavailable";

/// The stable identity of an adapter with these `DXGI_ADAPTER_DESC1` values.
pub fn identity(vendor_id: u32, device_id: u32, subsys_id: u32, revision: u32) -> String {
    format!("{IDENTITY_PREFIX}{vendor_id:04x}:{device_id:04x}:{subsys_id:08x}:{revision:02x}")
}

/// Whether `value` is a stable identity as the Runtime takes one (`is_dxgi_pci_identity`): the
/// prefix, then lowercase hexadecimal of exactly 4, 4, 8 and 2 digits separated by colons.
pub fn is_identity(value: &str) -> bool {
    let Some(rest) = value.strip_prefix(IDENTITY_PREFIX) else {
        return false;
    };
    let parts: Vec<&str> = rest.split(':').collect();
    parts.len() == IDENTITY_WIDTHS.len()
        && parts.iter().zip(IDENTITY_WIDTHS).all(|(part, width)| {
            part.len() == width && part.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// `D3DKMT_ADAPTERTYPE` as the kernel reports it (`KMTQAITYPE_ADAPTERTYPE`), with the bits the
/// Runtime reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterType(pub u32);

impl AdapterType {
    pub const RENDER_SUPPORTED: u32 = 1 << 0;
    pub const SOFTWARE_DEVICE: u32 = 1 << 2;
    pub const INDIRECT_DISPLAY_DEVICE: u32 = 1 << 6;
    pub const PARAVIRTUALIZED: u32 = 1 << 7;

    pub const fn render_supported(self) -> bool {
        (self.0 & Self::RENDER_SUPPORTED) != 0
    }

    pub const fn software_device(self) -> bool {
        (self.0 & Self::SOFTWARE_DEVICE) != 0
    }

    pub const fn indirect_display_device(self) -> bool {
        (self.0 & Self::INDIRECT_DISPLAY_DEVICE) != 0
    }

    pub const fn paravirtualized(self) -> bool {
        (self.0 & Self::PARAVIRTUALIZED) != 0
    }
}

/// An adapter's PCI location (`D3DKMT_ADAPTERADDRESS`): no part of the identity, but what tells
/// two physical adapters of one identity apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciAddress {
    pub bus: u32,
    pub device: u32,
    pub function: u32,
}

/// `bus:device.function`, as the Runtime's report writes it.
impl fmt::Display for PciAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:02x}:{:02x}.{:x}", self.bus, self.device, self.function)
    }
}

/// The kernel graphics call that failed while reading an adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelCall {
    OpenAdapterFromLuid,
    QueryAdapterType,
    CloseAdapter,
}

impl KernelCall {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenAdapterFromLuid => "D3DKMTOpenAdapterFromLuid",
            Self::QueryAdapterType => "D3DKMTQueryAdapterInfo(KMTQAITYPE_ADAPTERTYPE)",
            Self::CloseAdapter => "D3DKMTCloseAdapter",
        }
    }
}

/// What the kernel said of one adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kernel {
    /// The adapter type and, when the kernel gave it, the PCI location.
    Read {
        adapter_type: AdapterType,
        pci_address: Option<PciAddress>,
    },
    /// A kernel call failed with this NTSTATUS: the adapter cannot be shown to be physical.
    Failed { call: KernelCall, ntstatus: i32 },
}

/// One DXGI adapter as one listing read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// Its index in this listing's `EnumAdapters1` order.
    pub dxgi_index: u32,
    /// `DXGI_ADAPTER_DESC1::Description`, the name a person knows the card by.
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub subsys_id: u32,
    pub revision: u32,
    /// `DXGI_ADAPTER_DESC1::DedicatedVideoMemory`, in bytes.
    pub dedicated_video_memory: u64,
    /// `DXGI_ADAPTER_DESC1::Flags`.
    pub dxgi_flags: u32,
    pub kernel: Kernel,
}

impl Adapter {
    /// The stable identity `dxgi-pci:<vendor>:<device>:<subsys>:<rev>`.
    pub fn identity(&self) -> String {
        identity(self.vendor_id, self.device_id, self.subsys_id, self.revision)
    }

    /// The kernel's adapter type, when it could be read.
    pub const fn adapter_type(&self) -> Option<AdapterType> {
        match self.kernel {
            Kernel::Read { adapter_type, .. } => Some(adapter_type),
            Kernel::Failed { .. } => None,
        }
    }

    /// The PCI location, when the kernel gave it.
    pub const fn pci_address(&self) -> Option<PciAddress> {
        match self.kernel {
            Kernel::Read { pci_address, .. } => pci_address,
            Kernel::Failed { .. } => None,
        }
    }

    /// A software adapter, by DXGI's flag or by the kernel's type.
    pub fn software(&self) -> bool {
        (self.dxgi_flags & DXGI_ADAPTER_FLAG_SOFTWARE) != 0
            || self.adapter_type().is_some_and(AdapterType::software_device)
    }

    pub fn indirect_display(&self) -> bool {
        self.adapter_type().is_some_and(AdapterType::indirect_display_device)
    }

    pub fn paravirtualized(&self) -> bool {
        self.adapter_type().is_some_and(AdapterType::paravirtualized)
    }

    /// A physical adapter: the kernel's type was read, it renders, and it is neither a software,
    /// an indirect-display nor a paravirtualized device.
    pub fn physical(&self) -> bool {
        self.adapter_type().is_some_and(AdapterType::render_supported)
            && !self.software()
            && !self.indirect_display()
            && !self.paravirtualized()
    }
}

/// Why an adapter that is listed is not physical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotPhysical {
    /// The kernel's type could not be read.
    Unread { call: KernelCall, ntstatus: i32 },
    Paravirtualized,
    IndirectDisplay,
    /// The kernel's type says it does not render.
    NoRender,
}

/// What an adapter is to the graphics card option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// The Runtime resolves its identity to exactly this adapter: it can be chosen.
    Offered,
    /// Physical, but another physical adapter has its identity and the Runtime cannot tell them
    /// apart (`adapter_ambiguous`).
    Ambiguous,
    /// Physical, with the identity of another physical adapter that the Runtime takes in its
    /// place (the only one of them whose PCI location the kernel gave).
    Shadowed,
    /// Not physical, and no physical adapter has its identity (`adapter_not_physical`).
    NotPhysical(NotPhysical),
    /// A software adapter: never listed.
    Software,
    /// Not physical, while a physical adapter has its identity: another listing of a card listed
    /// already, never listed again.
    Copy,
}

impl Standing {
    /// Whether the option may name the adapter.
    pub fn offered(self) -> bool {
        self == Self::Offered
    }

    /// Whether the options page lists the adapter.
    pub fn shown(self) -> bool {
        !matches!(self, Self::Software | Self::Copy)
    }

    /// The standing in a person's words (no closing stop).
    pub fn said(self) -> Said {
        match self {
            Self::Offered => Said::new("可以选", "can be chosen"),
            Self::Ambiguous => Said::new(
                "不能选：这台电脑上有两块标识相同的显卡（同一型号），ActingCommand 分不清该用哪块",
                "cannot be chosen: two graphics cards on this computer have the same identity (the same model), and ActingCommand cannot tell which one to use",
            ),
            Self::Shadowed => Said::new(
                "不能选：另一块显卡和它标识相同，ActingCommand 只会用那一块",
                "cannot be chosen: another graphics card has the same identity, and ActingCommand would use that one",
            ),
            Self::NotPhysical(NotPhysical::Paravirtualized) => Said::new(
                "不能选：这是虚拟机里的显卡，不是这台电脑自己的",
                "cannot be chosen: this is a virtual machine's adapter, not a graphics card of this computer's own",
            ),
            Self::NotPhysical(NotPhysical::IndirectDisplay) => Said::new(
                "不能选：这是给虚拟显示器用的适配器，不是显卡",
                "cannot be chosen: this is an adapter for a virtual display, not a graphics card",
            ),
            Self::NotPhysical(NotPhysical::NoRender) => Said::new(
                "不能选：这块适配器不做图形计算",
                "cannot be chosen: this adapter does no rendering",
            ),
            Self::NotPhysical(NotPhysical::Unread { call, ntstatus }) => Said::new(
                format!(
                    "不能选：读不出它是不是这台电脑自己的显卡（{} 失败，NTSTATUS 0x{:08x}）",
                    call.as_str(),
                    ntstatus as u32
                ),
                format!(
                    "cannot be chosen: whether it is a graphics card of this computer's own cannot be read ({} failed, NTSTATUS 0x{:08x})",
                    call.as_str(),
                    ntstatus as u32
                ),
            ),
            Self::Software => Said::new(
                "软件适配器，不是显卡，不列出",
                "a software adapter, not a graphics card; not listed",
            ),
            Self::Copy => Said::new(
                "同一块显卡的另一个（非物理的）条目，不另外列出",
                "another, non-physical listing of a graphics card listed already; not listed again",
            ),
        }
    }
}

fn not_physical(adapter: &Adapter) -> NotPhysical {
    match adapter.kernel {
        Kernel::Failed { call, ntstatus } => NotPhysical::Unread { call, ntstatus },
        Kernel::Read { adapter_type, .. } if adapter_type.paravirtualized() => NotPhysical::Paravirtualized,
        Kernel::Read { adapter_type, .. } if adapter_type.indirect_display_device() => NotPhysical::IndirectDisplay,
        Kernel::Read { .. } => NotPhysical::NoRender,
    }
}

/// Each adapter's standing, in the listing's order (see the module's description).
pub fn standings(adapters: &[Adapter]) -> Vec<Standing> {
    adapters
        .iter()
        .map(|adapter| {
            if adapter.software() {
                return Standing::Software;
            }
            let identity = adapter.identity();
            let physical: Vec<&Adapter> = adapters
                .iter()
                .filter(|other| other.physical() && other.identity() == identity)
                .collect();
            if !adapter.physical() {
                return if physical.is_empty() {
                    Standing::NotPhysical(not_physical(adapter))
                } else {
                    Standing::Copy
                };
            }
            if physical.len() == 1 {
                return Standing::Offered;
            }
            let addressed: Vec<&Adapter> = physical
                .iter()
                .copied()
                .filter(|other| other.pci_address().is_some())
                .collect();
            match addressed.as_slice() {
                [only] if std::ptr::eq(*only, adapter) => Standing::Offered,
                [_] => Standing::Shadowed,
                _ => Standing::Ambiguous,
            }
        })
        .collect()
}

/// The adapters the options page lists, each with its standing, in the listing's order.
pub fn listing(adapters: &[Adapter]) -> Vec<(Adapter, Standing)> {
    adapters
        .iter()
        .cloned()
        .zip(standings(adapters))
        .filter(|(_, standing)| standing.shown())
        .collect()
}

/// The card ticked first when the option is ticked: of those that can be chosen, the one with the
/// most video memory of its own, the first in DXGI's order among equals; `None` when none can.
pub fn preselect(listed: &[(Adapter, Standing)]) -> Option<usize> {
    listed
        .iter()
        .enumerate()
        .filter(|(_, (_, standing))| standing.offered())
        .max_by(|(left_at, (left, _)), (right_at, (right, _))| {
            left.dedicated_video_memory
                .cmp(&right.dedicated_video_memory)
                .then(right_at.cmp(left_at))
        })
        .map(|(at, _)| at)
}

/// Whether a card with this much video memory of its own looks like integrated graphics.
pub fn looks_integrated(dedicated_video_memory: u64) -> bool {
    dedicated_video_memory < INTEGRATED_BELOW
}

/// What is said beside a chosen card that [`looks_integrated`]. The person decides.
pub fn integrated_warning() -> Said {
    Said::new(
        "这看起来是集成显卡：实测文字识别结果和 CPU 不完全一致，速度也没有优势，建议不要开。",
        "This looks like integrated graphics: in tests its text recognition did not fully match the CPU's and was no faster, so leaving this off is advised.",
    )
}

/// `bytes` in GB (counted in 1024s, as the window writes sizes), to a tenth, without a trailing
/// `.0`.
pub fn memory_gb(bytes: u64) -> String {
    let tenths = (bytes as f64 / GIB as f64 * 10.0).round() as u64;
    if tenths % 10 == 0 {
        format!("{} GB", tenths / 10)
    } else {
        format!("{}.{} GB", tenths / 10, tenths % 10)
    }
}

/// What is wrong with an identity's form.
pub fn identity_problem() -> Said {
    Said::new(
        "显卡的标识格式不对：应为 dxgi-pci:<厂商 4 位>:<设备 4 位>:<子系统 8 位>:<修订 2 位>，小写十六进制",
        "the graphics card's identity is not dxgi-pci:<vendor, 4>:<device, 4>:<subsystem, 8>:<revision, 2> in lowercase hexadecimal",
    )
}

/// The card a plan names, as this computer lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chosen {
    pub identity: String,
    pub name: String,
    pub dedicated_video_memory: u64,
}

/// The adapter of `identity` among `adapters`, when the option may name it ([`Standing::Offered`]);
/// else why not: no adapter has it, or the standing of the first that has it (a physical one's
/// before another listing's).
pub fn chosen(adapters: &[Adapter], identity: &str) -> Result<Chosen, Said> {
    if !is_identity(identity) {
        return Err(identity_problem().with(identity));
    }
    let mut first: Option<Standing> = None;
    for (adapter, standing) in adapters.iter().zip(standings(adapters)) {
        if adapter.identity() != identity {
            continue;
        }
        if standing.offered() {
            return Ok(Chosen {
                identity: identity.to_owned(),
                name: adapter.name.clone(),
                dedicated_video_memory: adapter.dedicated_video_memory,
            });
        }
        if first.is_none_or(|seen| !seen.shown() && standing.shown()) {
            first = Some(standing);
        }
    }
    Err(match first {
        None => Said::new("这台电脑上没有这块显卡", "this computer has no such graphics card"),
        Some(standing) => Said::new("这块显卡不能用来做文字识别", "this graphics card cannot be used for text recognition")
            .with_said(&standing.said()),
    }
    .with(identity))
}

/// Why the Runtime cannot use the card, by `gpu_unavailable_reason` (Runtime
/// `crates/vision-ffi/src/codes.rs` `GpuUnavailableReason`), in a person's words.
pub fn unavailable_said(reason: &str) -> Said {
    match reason {
        "adapter_missing" => Said::new("这台电脑上找不到这块显卡", "this computer has no such graphics card"),
        "adapter_not_physical" => Said::new(
            "它不是这台电脑自己的显卡（是虚拟的或软件的适配器）",
            "it is not a graphics card of this computer's own (a virtual or software adapter)",
        ),
        "adapter_ambiguous" => Said::new(
            "这台电脑上有两块标识相同的显卡，ActingCommand 分不清该用哪块",
            "two graphics cards on this computer have the same identity, and ActingCommand cannot tell which one to use",
        ),
        "directml_library_missing" => Said::new(
            "安装文件夹里没有 DirectML.dll",
            "DirectML.dll is missing from the install folder",
        ),
        "directml_library_mismatch" => Said::new(
            "安装文件夹里的 DirectML.dll 不是 Runtime 要的那一版，或者读不出",
            "DirectML.dll in the install folder is not the one the Runtime pins, or cannot be read",
        ),
        other => Said::new(
            format!("Runtime 说用不上（{other}）"),
            format!("the Runtime says it cannot be used ({other})"),
        ),
    }
}

/// A failure of the card chosen: `why`, the technical detail in brackets, and what to do. The
/// configuration is never rewritten to the CPU by the wizard.
fn cannot_use(why: &Said, detail: Option<String>) -> Said {
    let (zh_detail, en_detail) = match detail {
        Some(detail) if !detail.is_empty() => (format!("（{detail}）"), format!(" ({detail})")),
        _ => (String::new(), String::new()),
    };
    Said::new(
        format!(
            "选的显卡用不上：{}{zh_detail}。向导没有改成用 CPU：请重新运行安装向导，装到一个新文件夹，这次不要勾「用显卡做文字识别」",
            why.zh
        ),
        format!(
            "the graphics card chosen cannot be used: {}{en_detail}. The wizard did not switch to the CPU on its own: run the wizard again into a new folder, this time without \"Use the graphics card for text recognition\"",
            why.en
        ),
    )
}

/// What `check-config` resolved the identity to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The adapter's index in check-config's listing (which ONNX Runtime's `device_id` will be
    /// given at that moment; it may change between boots).
    pub dxgi_index: u64,
    /// Valid until the next boot; an observation.
    pub luid: Option<String>,
    pub pci_address: Option<String>,
}

impl fmt::Display for Resolved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "DXGI index {}", self.dxgi_index)?;
        if let Some(luid) = &self.luid {
            write!(formatter, ", LUID {luid}")?;
        }
        if let Some(address) = &self.pci_address {
            write!(formatter, ", PCI {address}")?;
        }
        Ok(())
    }
}

/// `check-config`'s answer `line` judged for the card `chosen` (Runtime
/// `apps/actingd/src/check_config.rs` `directml_report`; `contracts/actingd-check-config.md`,
/// "DirectML"): no `vision_gpu_unavailable` among the passed outcome's links; a
/// `report.vision.directml` whose `expected_identity` is `chosen`, whose `unavailable_reason` is
/// `null`, and whose `resolved` adapter is listed in its `adapters` with the identity `chosen`.
/// Anything else is a failure in a person's words that says to install again without the option.
pub fn judge_report(line: &Value, chosen: &str) -> Result<Resolved, Said> {
    // The note first: it carries the Runtime's own reason.
    if let Some(note) = line["outcome"]["causes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|cause| cause["code"].as_str() == Some(UNAVAILABLE_CODE))
    {
        let reason = note["values"]["gpu_unavailable_reason"].as_str().unwrap_or("?");
        return Err(cannot_use(&unavailable_said(reason), Some(outcome_text(note))));
    }
    let directml = &line["report"]["vision"]["directml"];
    if !directml.is_object() {
        return Err(cannot_use(
            &Said::new(
                "Runtime 的检查结果里没有显卡这一项",
                "the Runtime's check reports nothing of the graphics card",
            ),
            Some("report.vision.directml is missing".to_owned()),
        ));
    }
    let expected = directml["expected_identity"].as_str();
    if expected != Some(chosen) {
        return Err(cannot_use(
            &Said::new("Runtime 检查的不是选的那块显卡", "the Runtime checked another graphics card than the one chosen"),
            Some(format!("expected_identity {}, chosen {chosen}", expected.unwrap_or("?"))),
        ));
    }
    if let Some(reason) = directml["unavailable_reason"].as_str() {
        return Err(cannot_use(&unavailable_said(reason), Some(format!("unavailable_reason {reason}"))));
    }
    let resolved = &directml["resolved"];
    let Some(index) = resolved["dxgi_index"].as_u64() else {
        return Err(cannot_use(
            &Said::new("Runtime 没有找到选的那块显卡", "the Runtime did not find the graphics card chosen"),
            Some(format!("resolved {resolved}")),
        ));
    };
    let listed = directml["adapters"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|adapter| adapter["dxgi_index"].as_u64() == Some(index));
    let identity = listed.and_then(|adapter| adapter["identity"].as_str());
    if identity != Some(chosen) {
        return Err(cannot_use(
            &Said::new("Runtime 找到的不是选的那块显卡", "the Runtime found another graphics card than the one chosen"),
            Some(format!("DXGI index {index}: {}, chosen {chosen}", identity.unwrap_or("?"))),
        ));
    }
    Ok(Resolved {
        dxgi_index: index,
        luid: resolved["luid"].as_str().map(str::to_owned),
        pci_address: listed.and_then(|adapter| adapter["pci_address"].as_str()).map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RENDER: u32 = AdapterType::RENDER_SUPPORTED;

    fn card(index: u32, device: u32, memory_gib: u64, kernel: Kernel) -> Adapter {
        Adapter {
            dxgi_index: index,
            name: format!("Card {index}"),
            vendor_id: 0x10de,
            device_id: device,
            subsys_id: 0x8a1f_1043,
            revision: 0xa1,
            dedicated_video_memory: memory_gib * GIB,
            dxgi_flags: 0,
            kernel,
        }
    }

    fn read(bits: u32, address: Option<u32>) -> Kernel {
        Kernel::Read {
            adapter_type: AdapterType(bits),
            pci_address: address.map(|bus| PciAddress { bus, device: 0, function: 0 }),
        }
    }

    /// Microsoft's Basic Render Driver: software by DXGI's flag.
    fn basic_render(index: u32) -> Adapter {
        Adapter {
            dxgi_index: index,
            name: "Microsoft Basic Render Driver".into(),
            vendor_id: 0x1414,
            device_id: 0x008c,
            subsys_id: 0,
            revision: 0,
            dedicated_video_memory: 0,
            dxgi_flags: DXGI_ADAPTER_FLAG_SOFTWARE,
            kernel: read(RENDER | AdapterType::SOFTWARE_DEVICE, None),
        }
    }

    #[test]
    fn the_identity_is_the_runtimes() {
        assert_eq!(identity(0x10de, 0x2504, 0x8a1f_1043, 0xa1), "dxgi-pci:10de:2504:8a1f1043:a1");
        assert_eq!(identity(0x8086, 0x46a6, 0, 0x0c), "dxgi-pci:8086:46a6:00000000:0c");
        assert!(is_identity("dxgi-pci:10de:2504:8a1f1043:a1"));
        assert!(is_identity(&identity(u32::from(u16::MAX), 0, u32::MAX, 0xff)));
        for wrong in [
            "dxgi-pci:10DE:2504:8a1f1043:a1",
            "dxgi-pci:10de:2504:8a1f1043",
            "dxgi-pci:10de:2504:8a1f1043:a1:00",
            "dxgi-pci:10de:250:8a1f1043:a1",
            "dxgi-pci:10de:2504:8a1f10430:a1",
            "pci:10de:2504:8a1f1043:a1",
            "dxgi-pci:10de:2504:8a1f1043:g1",
            "0",
            "",
        ] {
            assert!(!is_identity(wrong), "{wrong}");
        }
    }

    #[test]
    fn a_physical_card_of_its_own_identity_is_offered_and_software_is_never_listed() {
        let adapters = vec![card(0, 0x2504, 12, read(RENDER | 2, Some(1))), basic_render(1)];
        assert_eq!(standings(&adapters), vec![Standing::Offered, Standing::Software]);
        let listed = listing(&adapters);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0.name, "Card 0");
        // Software by the kernel's bit alone.
        let mut kernel_software = card(0, 0x2504, 1, read(RENDER | AdapterType::SOFTWARE_DEVICE, Some(1)));
        kernel_software.dxgi_flags = 0;
        assert_eq!(standings(&[kernel_software]), vec![Standing::Software]);
    }

    #[test]
    fn another_listing_of_a_physical_card_is_not_listed_again() {
        // An indirect-display copy that reports itself as a render adapter, with the card's
        // identity, and a paravirtualized one: the Runtime keeps the physical card.
        let adapters = vec![
            card(0, 0x2504, 12, read(RENDER, Some(1))),
            card(1, 0x2504, 12, read(RENDER | AdapterType::INDIRECT_DISPLAY_DEVICE, None)),
            card(2, 0x2504, 12, read(RENDER | AdapterType::PARAVIRTUALIZED, Some(2))),
        ];
        assert_eq!(standings(&adapters), vec![Standing::Offered, Standing::Copy, Standing::Copy]);
        assert_eq!(listing(&adapters).len(), 1);
        assert_eq!(chosen(&adapters, &adapters[0].identity()).unwrap().name, "Card 0");
    }

    #[test]
    fn two_physical_cards_of_one_identity_follow_the_runtimes_pci_rule() {
        // Both with a PCI location: ambiguous, both greyed.
        let both = vec![card(0, 0x2504, 8, read(RENDER, Some(1))), card(1, 0x2504, 8, read(RENDER, Some(2)))];
        assert_eq!(standings(&both), vec![Standing::Ambiguous, Standing::Ambiguous]);
        assert_eq!(listing(&both).len(), 2);
        assert_eq!(preselect(&listing(&both)), None);
        let refused = chosen(&both, &both[0].identity()).unwrap_err();
        assert!(refused.en.contains("cannot tell which"), "{refused}");
        // Neither: ambiguous too.
        let neither = vec![card(0, 0x2504, 8, read(RENDER, None)), card(1, 0x2504, 8, read(RENDER, None))];
        assert_eq!(standings(&neither), vec![Standing::Ambiguous, Standing::Ambiguous]);
        // Exactly one: the Runtime takes that one.
        let one = vec![card(0, 0x2504, 8, read(RENDER, None)), card(1, 0x2504, 8, read(RENDER, Some(2)))];
        assert_eq!(standings(&one), vec![Standing::Shadowed, Standing::Offered]);
        let listed = listing(&one);
        assert_eq!(preselect(&listed), Some(1));
        assert_eq!(chosen(&one, &one[0].identity()).unwrap().name, "Card 1");
    }

    #[test]
    fn an_adapter_that_is_not_physical_says_why() {
        let adapters = vec![
            card(0, 0x0001, 4, read(RENDER | AdapterType::PARAVIRTUALIZED, Some(1))),
            card(1, 0x0002, 0, read(RENDER | AdapterType::INDIRECT_DISPLAY_DEVICE, None)),
            card(2, 0x0003, 0, read(2, Some(3))),
            card(3, 0x0004, 4, Kernel::Failed { call: KernelCall::QueryAdapterType, ntstatus: 0xc000_0001_u32 as i32 }),
        ];
        let standings = standings(&adapters);
        assert_eq!(
            standings,
            vec![
                Standing::NotPhysical(NotPhysical::Paravirtualized),
                Standing::NotPhysical(NotPhysical::IndirectDisplay),
                Standing::NotPhysical(NotPhysical::NoRender),
                Standing::NotPhysical(NotPhysical::Unread {
                    call: KernelCall::QueryAdapterType,
                    ntstatus: 0xc000_0001_u32 as i32
                }),
            ]
        );
        // Listed, greyed, none to tick.
        assert_eq!(listing(&adapters).len(), 4);
        assert_eq!(preselect(&listing(&adapters)), None);
        assert!(standings[3].said().en.contains("NTSTATUS 0xc0000001"), "{}", standings[3].said());
        assert!(standings.iter().all(|standing| !standing.offered() && standing.shown()));
        let refused = chosen(&adapters, &adapters[0].identity()).unwrap_err();
        assert!(refused.zh.contains("虚拟机"), "{refused}");
    }

    #[test]
    fn the_card_with_the_most_memory_of_its_own_is_ticked_first() {
        let adapters = vec![
            card(0, 0x46a6, 0, read(RENDER, Some(0))),
            card(1, 0x2504, 6, read(RENDER, Some(1))),
            card(2, 0x2505, 12, read(RENDER, None)),
            card(3, 0x2506, 12, read(RENDER, Some(3))),
            basic_render(4),
        ];
        let listed = listing(&adapters);
        assert_eq!(listed.len(), 4);
        // Two of 12 GiB: the first in DXGI's order.
        assert_eq!(preselect(&listed), Some(2));
        // Only the integrated one can be chosen: it is still ticked first (the person decides).
        let integrated = vec![card(0, 0x46a6, 0, read(RENDER, Some(0))), basic_render(1)];
        assert_eq!(preselect(&listing(&integrated)), Some(0));
        assert_eq!(preselect(&[]), None);
    }

    #[test]
    fn under_two_gib_of_its_own_looks_integrated() {
        assert!(looks_integrated(0));
        assert!(looks_integrated(128 << 20));
        assert!(looks_integrated(2 * GIB - 1));
        assert!(!looks_integrated(2 * GIB));
        assert!(integrated_warning().zh.starts_with("这看起来是集成显卡"));
        assert_eq!(memory_gb(0), "0 GB");
        assert_eq!(memory_gb(128 << 20), "0.1 GB");
        assert_eq!(memory_gb(512 << 20), "0.5 GB");
        assert_eq!(memory_gb(12 * GIB), "12 GB");
        assert_eq!(memory_gb(4095 << 20), "4 GB");
        assert_eq!(memory_gb(6 * GIB + (GIB / 2)), "6.5 GB");
    }

    #[test]
    fn a_plans_card_must_be_one_this_computer_offers() {
        let adapters = vec![card(0, 0x2504, 12, read(RENDER, Some(1))), basic_render(1)];
        let identity = adapters[0].identity();
        assert_eq!(
            chosen(&adapters, &identity),
            Ok(Chosen { identity: identity.clone(), name: "Card 0".into(), dedicated_video_memory: 12 * GIB })
        );
        let missing = chosen(&adapters, "dxgi-pci:1002:73bf:00000000:c1").unwrap_err();
        assert!(missing.en.contains("no such graphics card"), "{missing}");
        let software = chosen(&adapters, &adapters[1].identity()).unwrap_err();
        assert!(software.en.contains("software adapter"), "{software}");
        assert!(chosen(&adapters, "DXGI-PCI:10de").is_err());
    }

    const CHOSEN: &str = "dxgi-pci:10de:2504:8a1f1043:a1";

    /// A passed check-config line with `vision.directml` as the contract's example writes it
    /// (Runtime `contracts/actingd-check-config.md`, "DirectML"), the other members left out.
    fn passed() -> Value {
        json!({
            "schema_version": "actingcommand.actingd.check-config.v2",
            "outcome": {"schema_version": "actingcommand.outcome.v1", "code": "check_config_passed", "category": "success", "values": {}},
            "report": {"vision": {"root": "X:\\Root\\vision", "execution_provider": "directml", "directml": {
                "expected_identity": CHOSEN, "gpu_fallback": "cpu",
                "adapters": [
                    {"dxgi_index": 0, "identity": CHOSEN, "luid": "0x0000000000014ad3", "physical": true,
                     "software": false, "indirect_display": false, "paravirtualized": false, "pci_address": "01:00.0"},
                    {"dxgi_index": 1, "identity": "dxgi-pci:1414:008c:00000000:00", "luid": "0x0000000000014b01", "physical": false,
                     "software": true, "indirect_display": false, "paravirtualized": false, "pci_address": null}
                ],
                "resolved": {"dxgi_index": 0, "luid": "0x0000000000014ad3"}, "unavailable_reason": null,
                "runtime_pin": {"onnxruntime": "ok", "providers_shared": "ok", "directml": "ok"},
                "nn_execution_provider": "cpu"
            }}}
        })
    }

    #[test]
    fn check_config_must_resolve_the_card_chosen() {
        let resolved = judge_report(&passed(), CHOSEN).expect("resolved");
        assert_eq!(
            resolved,
            Resolved { dxgi_index: 0, luid: Some("0x0000000000014ad3".into()), pci_address: Some("01:00.0".into()) }
        );
        assert_eq!(resolved.to_string(), "DXGI index 0, LUID 0x0000000000014ad3, PCI 01:00.0");
    }

    #[test]
    fn a_card_check_config_cannot_use_fails_the_step_with_what_to_do() {
        // The note, as a passed outcome carries it: the reason in words, and the advice.
        let mut noted = passed();
        noted["outcome"]["causes"] = json!([{"code": "vision_gpu_unavailable", "category": "warning", "relation": "note",
            "values": {"execution_provider": "directml", "gpu_unavailable_reason": "adapter_missing", "gpu_fallback": "cpu", "stage": "config_vision"}}]);
        noted["report"]["vision"]["directml"]["resolved"] = Value::Null;
        noted["report"]["vision"]["directml"]["unavailable_reason"] = json!("adapter_missing");
        let refused = judge_report(&noted, CHOSEN).unwrap_err();
        assert!(refused.zh.contains("这台电脑上找不到这块显卡"), "{refused}");
        assert!(refused.zh.contains("装到一个新文件夹") && refused.zh.contains("不要勾「用显卡做文字识别」"), "{refused}");
        assert!(refused.en.contains("did not switch to the CPU"), "{refused}");
        assert!(refused.en.contains("vision_gpu_unavailable"), "{refused}");
        // A note with the adapter resolved (DirectML.dll off its pin) fails too.
        let mut library = passed();
        library["outcome"]["causes"] = json!([{"code": "vision_gpu_unavailable", "category": "warning", "relation": "note",
            "values": {"gpu_unavailable_reason": "directml_library_mismatch"}}]);
        let refused = judge_report(&library, CHOSEN).unwrap_err();
        assert!(refused.en.contains("DirectML.dll in the install folder"), "{refused}");
        // Each other way the report does not show the card chosen resolved.
        let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
            ("no directml", Box::new(|line: &mut Value| {
                line["report"]["vision"].as_object_mut().unwrap().remove("directml");
            })),
            ("no report", Box::new(|line: &mut Value| {
                line.as_object_mut().unwrap().remove("report");
            })),
            ("another expected", Box::new(|line: &mut Value| {
                line["report"]["vision"]["directml"]["expected_identity"] = json!("dxgi-pci:1002:73bf:00000000:c1")
            })),
            ("a reason without the note", Box::new(|line: &mut Value| {
                line["report"]["vision"]["directml"]["unavailable_reason"] = json!("adapter_ambiguous")
            })),
            ("none resolved", Box::new(|line: &mut Value| line["report"]["vision"]["directml"]["resolved"] = Value::Null)),
            ("resolved to another", Box::new(|line: &mut Value| {
                line["report"]["vision"]["directml"]["resolved"]["dxgi_index"] = json!(1)
            })),
            ("resolved to none listed", Box::new(|line: &mut Value| {
                line["report"]["vision"]["directml"]["resolved"]["dxgi_index"] = json!(7)
            })),
        ];
        for (name, change) in cases {
            let mut line = passed();
            change(&mut line);
            let refused = judge_report(&line, CHOSEN).unwrap_err();
            assert!(refused.en.contains("run the wizard again into a new folder"), "{name}: {refused}");
        }
        // A reason the wizard does not know is still a failure, named by its token.
        assert!(unavailable_said("provider_register_failed").en.contains("provider_register_failed"));
    }
}
