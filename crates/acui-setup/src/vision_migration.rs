// SPDX-License-Identifier: GPL-3.0-only
//! Vision migration (Workflow #359 items 1 and 6, #360 §10): the v0.3 vision
//! provider manifest a configuration names becomes the model-folder layout
//! under `<root>\vision`, and the configuration's `vision_provider_manifest`
//! becomes a `vision` section.
//!
//! - Each OCR or NN section's files are copied into `vision\models\<model_ref>\`
//!   under the names the folder rule reads, every copy checked against the
//!   manifest's sha256, and the OCR composite against its `model_sha256`.
//! - ONNX Runtime ends up in `vision\ort\`; a library already there is used as
//!   it is.
//! - Nothing is moved or deleted: the manifest, the flat model files, every
//!   other manifest and the source runtime libraries stay, so a generation that
//!   still names them keeps working after a switch back.
//!
//! `plan` only reads; `Plan::apply` writes, through `<name>.new` and a rename.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::verify::{hex, sha256_file, Report, MISMATCH};

const MANIFEST_SCHEMA: &str = "actingcommand.vision_provider_artifacts.v0.3";
/// The description an NN folder needs: the folder rule's family for `model.onnx`.
const NN_DESCRIPTION: &[u8] =
    b"{\"schema_version\":\"actingcommand.vision_model.v1\",\"family\":\"onnx-classify\"}\n";

/// One file the migration places, and whether the identical file is already there.
struct Placement {
    from: PathBuf,
    to: PathBuf,
    sha256: String,
    present: bool,
}

/// What a migration does, computed without writing anything.
pub struct Plan {
    source: PathBuf,
    vision: Value,
    folders: Vec<PathBuf>,
    files: Vec<Placement>,
    /// An NN folder's `model.json`, and whether the same bytes are already there.
    descriptions: Vec<(PathBuf, bool)>,
    ort: Vec<Placement>,
    ort_in_place: Vec<PathBuf>,
}

#[cfg(windows)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn reparse(_metadata: &fs::Metadata) -> bool {
    false
}

fn hash(path: &Path) -> Result<String, String> {
    sha256_file(path).map_err(|error| format!("Cannot hash {}: {error}", path.display()))
}

/// Whether a regular file is at `path`; a link or anything else there stops the plan.
fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
        Ok(metadata)
            if metadata.is_file() && !metadata.file_type().is_symlink() && !reparse(&metadata) =>
        {
            Ok(true)
        }
        Ok(_) => Err(format!(
            "视觉文件位置被非普通文件占用 / A vision file's place holds something that is not a regular file: {}",
            path.display()
        )),
    }
}

/// A folder the migration writes into: absent, or a plain directory.
fn plain_dir(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot inspect {}: {error}", path.display())),
        Ok(metadata)
            if metadata.is_dir() && !metadata.file_type().is_symlink() && !reparse(&metadata) =>
        {
            Ok(())
        }
        Ok(_) => Err(format!(
            "视觉文件夹不是普通文件夹 / A vision folder is not a plain folder: {}",
            path.display()
        )),
    }
}

/// `model_ref` as a model folder's name (#360 §2.5), one path component.
fn folder_name(section: &Value, label: &str) -> Result<String, String> {
    let name = section
        .get("model_ref")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("视觉清单的 {label} 缺少 model_ref / The manifest's {label} section has no model_ref"))?;
    let valid = !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        });
    if !valid {
        return Err(format!(
            "model_ref 不能用作模型文件夹名 / model_ref is not a valid model folder name: {name:?}"
        ));
    }
    Ok(name.to_string())
}

fn sha_field(section: &Value, key: &str, label: &str) -> Result<String, String> {
    let value = section
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            format!("视觉清单的 {label} 缺少 {key} / The manifest's {label} section has no {key}")
        })?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "{label} {key} 不是 sha256 / is not a sha256: {value}"
        ));
    }
    Ok(value)
}

/// A manifest path, relative ones against the manifest's own folder.
fn source_path(section: &Value, key: &str, label: &str, base: &Path) -> Result<PathBuf, String> {
    let text = section.get(key).and_then(Value::as_str).ok_or_else(|| {
        format!("视觉清单的 {label} 缺少 {key} / The manifest's {label} section has no {key}")
    })?;
    let path = Path::new(text);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

/// The OCR composite identity (#360 §3.3), over the three per-file sha256s.
fn composite(detector: &str, recognizer: &str, dictionary: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"actingcommand.ppocr-model-set.v1\0");
    for (label, digest) in [
        ("detector", detector),
        ("recognizer", recognizer),
        ("dictionary", dictionary),
    ] {
        hasher.update(label.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(b"classifier\0none\0");
    hex(&hasher.finalize())
}

/// One model file: the source matches the manifest, and the target is either
/// absent (to be copied) or already byte-identical (reused). A different file
/// there stops the plan and names it.
fn placement(from: PathBuf, to: PathBuf, sha256: String) -> Result<Placement, String> {
    if !regular(&from)? {
        return Err(format!(
            "视觉清单指向的文件不存在 / A file the manifest names is missing: {}",
            from.display()
        ));
    }
    let actual = hash(&from)?;
    if actual != sha256 {
        return Err(format!(
            "{MISMATCH}: {} sha256 应为 / expected {sha256}，实为 / actual {actual}",
            from.display()
        ));
    }
    let present = regular(&to)?;
    if present && hash(&to)? != sha256 {
        return Err(format!(
            "模型文件夹已有不同的文件，请移开后重试 / The model folder already holds a different file; move it aside and run again: {}",
            to.display()
        ));
    }
    Ok(Placement {
        from,
        to,
        sha256,
        present,
    })
}

/// One section's execution provider, and its CUDA device when it has one.
fn execution(section: &Value, label: &str) -> Result<(String, Option<Value>), String> {
    let provider = section
        .get("execution_provider")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("视觉清单的 {label} 没有明确的 execution_provider / The manifest's {label} section states no execution_provider"))?;
    let device = section.get("cuda_device").filter(|value| !value.is_null());
    match (provider, device) {
        ("cpu", None) => Ok(("cpu".to_string(), None)),
        ("cuda", Some(device)) => Ok(("cuda".to_string(), Some(device.clone()))),
        ("cuda", None) => Err(format!(
            "{label} 选了 cuda 却没有 cuda_device / {label} selects cuda without a cuda_device"
        )),
        ("cpu", Some(_)) => Err(format!(
            "{label} 选了 cpu 却给了 cuda_device / {label} selects cpu with a cuda_device"
        )),
        (other, _) => Err(format!(
            "{label} 的 execution_provider 只能是 cpu 或 cuda / {label} execution_provider must be cpu or cuda, not {other}"
        )),
    }
}

/// The migration a configuration needs, read-only: `None` when it names no
/// v0.3 manifest. `root` is the installation root; `document`'s paths are
/// absolute (`generations::rebase_config`).
pub fn plan(root: &Path, document: &Value) -> Result<Option<Plan>, String> {
    let Some(manifest) = document
        .get("vision_provider_manifest")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    if document.get("vision").is_some_and(|value| !value.is_null()) {
        return Err("配置同时有 vision 与 vision_provider_manifest / The configuration has both vision and vision_provider_manifest".into());
    }
    let source = PathBuf::from(
        manifest
            .as_str()
            .ok_or("vision_provider_manifest is not a path")?,
    );
    let base = source
        .parent()
        .ok_or("Provider manifest has no parent")?
        .to_path_buf();
    let text: Value = serde_json::from_slice(&acui_installation::read_bounded(
        &source,
        acui_installation::MAX_MATERIAL_BYTES,
    )?)
    .map_err(|error| {
        format!(
            "视觉清单无法解析 / The vision provider manifest is unreadable: {}: {error}",
            source.display()
        )
    })?;
    if text.get("schema_version").and_then(Value::as_str) != Some(MANIFEST_SCHEMA) {
        return Err(format!(
            "视觉清单不是 v0.3，无法迁移 / The vision provider manifest is not {MANIFEST_SCHEMA} and cannot be migrated: {} ({})",
            source.display(),
            text["schema_version"]
        ));
    }
    let models = root.join("vision").join("models");
    plain_dir(&root.join("vision"))?;
    plain_dir(&models)?;
    let ocr = text
        .get("fastdeploy_ppocr")
        .filter(|value| !value.is_null());
    let nn = text.get("onnxruntime").filter(|value| !value.is_null());
    if ocr.is_none() && nn.is_none() {
        return Err(format!(
            "视觉清单没有任何段 / The vision provider manifest has no section: {}",
            source.display()
        ));
    }
    let mut plan = Plan {
        source: source.clone(),
        vision: Value::Null,
        folders: Vec::new(),
        files: Vec::new(),
        descriptions: Vec::new(),
        ort: Vec::new(),
        ort_in_place: Vec::new(),
    };
    let mut executions = Vec::new();
    let mut libraries = Vec::new();
    if let Some(section) = ocr {
        let label = "fastdeploy_ppocr";
        if section
            .get("classifier_model_path")
            .is_some_and(|value| !value.is_null())
        {
            return Err("视觉清单配置了方向分类器，模型文件夹不支持 / The manifest configures an angle classifier, which model folders do not support (classifier_model_path)".into());
        }
        let name = folder_name(section, label)?;
        let folder = models.join(&name);
        plain_dir(&folder)?;
        let mut digests = Vec::new();
        for (path_key, sha_key, file) in [
            ("detector_model_path", "detector_model_sha256", "det.onnx"),
            (
                "recognizer_model_path",
                "recognizer_model_sha256",
                "rec.onnx",
            ),
            ("dictionary_path", "dictionary_sha256", "keys.txt"),
        ] {
            let sha256 = sha_field(section, sha_key, label)?;
            digests.push(sha256.clone());
            plan.files.push(placement(
                source_path(section, path_key, label, &base)?,
                folder.join(file),
                sha256,
            )?);
        }
        let declared = sha_field(section, "model_sha256", label)?;
        let actual = composite(&digests[0], &digests[1], &digests[2]);
        if actual != declared {
            return Err(format!(
                "{MISMATCH}: {name} 的组合摘要应为 / composite should be {declared}，实为 / is {actual}"
            ));
        }
        plan.folders.push(folder);
        executions.push((label, execution(section, label)?));
        if let Some(paths) = section
            .get("runtime_library_paths")
            .and_then(Value::as_array)
        {
            for path in paths {
                let text = path
                    .as_str()
                    .ok_or("runtime_library_paths holds a value that is not a path")?;
                libraries.push(PathBuf::from(text));
            }
        }
        if let Some(path) = section.get("runtime_library_path").and_then(Value::as_str) {
            libraries.push(PathBuf::from(path));
        }
    }
    if let Some(section) = nn {
        let label = "onnxruntime";
        let name = folder_name(section, label)?;
        let folder = models.join(&name);
        if plan.folders.contains(&folder) {
            return Err(format!(
                "OCR 与 NN 用了同一个模型文件夹 / The OCR and NN sections name the same model folder: {name}"
            ));
        }
        plain_dir(&folder)?;
        plan.files.push(placement(
            source_path(section, "model_path", label, &base)?,
            folder.join("model.onnx"),
            sha_field(section, "model_sha256", label)?,
        )?);
        let description = folder.join("model.json");
        let present = regular(&description)?;
        if present
            && fs::read(&description)
                .map_err(|error| format!("Cannot read {}: {error}", description.display()))?
                != NN_DESCRIPTION
        {
            return Err(format!(
                "模型文件夹已有不同的描述文件，请移开后重试 / The model folder already holds a different description; move it aside and run again: {}",
                description.display()
            ));
        }
        plan.descriptions.push((description, present));
        plan.folders.push(folder);
        executions.push((label, execution(section, label)?));
        if let Some(path) = section.get("runtime_library_path").and_then(Value::as_str) {
            libraries.push(PathBuf::from(path));
        }
    }
    let (_, (provider, device)) = executions[0].clone();
    if let Some((label, _)) = executions.iter().find(|(_, (other, _))| *other != provider) {
        return Err(format!(
            "OCR 与 NN 的 execution_provider 不同，一个进程只能选一个 / {label}'s execution_provider differs; one process takes one"
        ));
    }
    plan.vision = match device {
        Some(device) => json!({"execution_provider": provider, "cuda_device": device}),
        None => json!({"execution_provider": provider}),
    };
    plan_runtime(root, &base, libraries, &mut plan)?;
    Ok(Some(plan))
}

/// ONNX Runtime under `<root>\vision\ort\`: a library already there stays as it
/// is; one elsewhere (a slot copy, another folder) is copied there when absent,
/// an identical one there is used, a different one stops the plan.
fn plan_runtime(
    root: &Path,
    base: &Path,
    libraries: Vec<PathBuf>,
    plan: &mut Plan,
) -> Result<(), String> {
    let ort = root.join("vision").join("ort");
    plain_dir(&ort)?;
    let ort_canonical = fs::canonicalize(&ort).ok();
    let mut seen = Vec::new();
    for library in libraries {
        let library = if library.is_absolute() {
            library
        } else {
            base.join(library)
        };
        let actual = fs::canonicalize(&library).map_err(|error| {
            format!(
                "视觉清单指向的运行库不存在 / A runtime library the manifest names is unavailable: {}: {error}",
                library.display()
            )
        })?;
        if seen.contains(&actual) {
            continue;
        }
        seen.push(actual.clone());
        let name = actual
            .file_name()
            .ok_or_else(|| format!("Runtime library has no file name: {}", actual.display()))?
            .to_os_string();
        if ort_canonical.as_deref() == actual.parent() {
            plan.ort_in_place.push(ort.join(&name));
            continue;
        }
        let to = ort.join(&name);
        let sha256 = hash(&actual)?;
        let present = regular(&to)?;
        if present && hash(&to)? != sha256 {
            return Err(format!(
                "安装根的 vision\\ort 已有不同的 {}，请核对后重试 / <root>\\vision\\ort already holds a different {}: {} differs from {}",
                to.display(),
                name.to_string_lossy(),
                to.display(),
                actual.display()
            ));
        }
        plan.ort.push(Placement {
            from: actual,
            to,
            sha256,
            present,
        });
    }
    let dll = ort.join("onnxruntime.dll");
    let results = plan.ort_in_place.contains(&dll)
        || plan.ort.iter().any(|placement| placement.to == dll)
        || regular(&dll)?;
    if !results {
        return Err(format!(
            "迁移后 {} 不存在：请把 ONNX Runtime 放到那里后重试 / {} would not exist after the migration: put ONNX Runtime there and run again",
            dll.display(),
            dll.display()
        ));
    }
    Ok(())
}

impl Plan {
    /// What a real run does, one line each.
    pub fn describe(&self, report: Report<'_>) -> Result<(), String> {
        report.line(&format!(
            "视觉迁移 / Vision migration from {}（清单与平铺的模型文件保留不动 / the manifest and the flat model files stay）",
            self.source.display()
        ))?;
        for placement in self.files.iter().chain(&self.ort) {
            report.line(&format!(
                "  {} {} ← {}（sha256 {}）",
                if placement.present {
                    "已有相同文件，复用 / identical, reused:"
                } else {
                    "复制 / copy:"
                },
                placement.to.display(),
                placement.from.display(),
                placement.sha256
            ))?;
        }
        for (description, present) in &self.descriptions {
            report.line(&format!(
                "  {} {}",
                if *present {
                    "已有相同描述，复用 / identical description, reused:"
                } else {
                    "写入描述 / description written:"
                },
                description.display()
            ))?;
        }
        for library in &self.ort_in_place {
            report.line(&format!(
                "  已在安装根，不动 / already under the root, untouched: {}",
                library.display()
            ))?;
        }
        report.line(&format!(
            "  配置 / configuration: vision_provider_manifest 删除 / removed; \"vision\": {}",
            self.vision
        ))
    }

    /// Copies what the plan needs, each file through `<name>.new`, hashed, then
    /// renamed. Every file already in place is checked again first.
    pub fn apply(&self, report: Report<'_>) -> Result<(), String> {
        self.describe(report)?;
        for placement in self.files.iter().chain(&self.ort) {
            if placement.present {
                if hash(&placement.to)? != placement.sha256 {
                    return Err(format!(
                        "{MISMATCH}: {} changed after planning",
                        placement.to.display()
                    ));
                }
                continue;
            }
            place(&placement.from, &placement.to, &placement.sha256)?;
        }
        for (description, present) in &self.descriptions {
            if *present {
                continue;
            }
            let part = with_new(description);
            if regular(&part)? {
                return Err(format!("Unfinished copy is in the way: {}", part.display()));
            }
            crate::generations::write_new(&part, NN_DESCRIPTION)?;
            rename(&part, description)?;
        }
        report.line("视觉模型文件夹已就绪 / Vision model folders are in place")
    }

    /// The configuration without `vision_provider_manifest`, with `vision`.
    pub fn rewrite(&self, document: &mut Value) -> Result<(), String> {
        let object = document
            .as_object_mut()
            .ok_or("Configuration must be an object")?;
        object.remove("vision_provider_manifest");
        object.insert("vision".to_string(), self.vision.clone());
        Ok(())
    }
}

fn with_new(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".new");
    path.with_file_name(name)
}

fn rename(from: &Path, to: &Path) -> Result<(), String> {
    if regular(to)? {
        return Err(format!(
            "目标已出现文件 / A file appeared at the target: {}",
            to.display()
        ));
    }
    fs::rename(from, to).map_err(|error| {
        format!(
            "Cannot rename {} to {}: {error}",
            from.display(),
            to.display()
        )
    })
}

fn place(from: &Path, to: &Path, sha256: &str) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    }
    let part = with_new(to);
    if regular(&part)? {
        return Err(format!(
            "未完成的复制挡住了位置，请移开后重试 / An unfinished copy is in the way; move it aside and run again: {}",
            part.display()
        ));
    }
    fs::copy(from, &part).map_err(|error| {
        format!(
            "复制失败 / copy failed: {} → {}: {error}",
            from.display(),
            part.display()
        )
    })?;
    let actual = hash(&part)?;
    if actual != sha256 {
        return Err(format!(
            "{MISMATCH}: {} sha256 应为 / expected {sha256}，实为 / actual {actual}",
            part.display()
        ));
    }
    rename(&part, to)
}
