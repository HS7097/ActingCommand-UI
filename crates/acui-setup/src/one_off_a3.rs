// SPDX-License-Identifier: GPL-3.0-only
// one-off (to be reverted): Workflow #288 A3 evidence on CI. The ten sealed BA packs of the
// public umbrella bundle (536f048a, bundles/bluearchive-bundle-3ff697b.zip) are unsealed as RT
// A2b's E3 did (six derived files removed), admitted by the real `actinglab package digest`
// (RT tools built from the exact tree of RT main 3b15f86f), and bundled by `actinglab package
// bundle`; that output directory is zipped at its root, the bundle v2 ZIP the resource line
// publishes. The installer's own `bundle::local` + `Bundle::lay_out` then lays it out; each
// digest-named directory is confirmed by `package digest`; a corrupted file takes the
// `<digest>.broken-<unix>` path; a leftover `.part` and a corrupt bundle are kept, never
// deleted; `actingd check-config` admits the default pack's directory; and the v1 bundle still
// lays out as before, next to the v2 directories.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::bundle::{self, Bundle};
use crate::verify::{hex, sha256_file};

const V1_SHA256: &str = "df524eac6290c4c0b435f3872a89e12dbe4f49e37555fa102bf96dd71b696af8";
const SOURCE_REPOSITORY: &str = "HS7097/ActingCommand-Resources-BlueArchive";
const CONTENT_DIRECTORY_V1: &str = "actingcommand.package.content-directory.v1";

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set")))
}

struct Log(fs::File);

impl Log {
    fn say(&mut self, line: impl AsRef<str>) {
        let line = format!("ONE-OFF-A3 {}", line.as_ref());
        eprintln!("{line}");
        writeln!(self.0, "{line}").expect("write the one-off log");
    }
}

fn read_zip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("open zip");
    let mut entries = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("zip entry");
        if file.is_dir() {
            continue;
        }
        let mut content = Vec::new();
        file.read_to_end(&mut content).expect("read zip entry");
        entries.insert(file.name().to_owned(), content);
    }
    entries
}

fn write_tree(root: &Path, entries: &BTreeMap<String, Vec<u8>>) {
    for (path, bytes) in entries {
        let target = path.split('/').fold(root.to_path_buf(), |parent, part| parent.join(part));
        fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
        fs::write(target, bytes).expect("write entry");
    }
}

fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("read dir") {
            let entry = entry.expect("dir entry");
            let name = entry.file_name().to_str().expect("utf-8 name").to_owned();
            let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            if entry.file_type().expect("file type").is_dir() {
                pending.push((entry.path(), relative));
            } else {
                entries.insert(relative, fs::read(entry.path()).expect("read file"));
            }
        }
    }
    entries
}

fn write_zip(path: &Path, entries: &BTreeMap<String, Vec<u8>>) {
    let mut writer = zip::ZipWriter::new(fs::File::create(path).expect("create zip"));
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(name.as_str(), options).expect("zip entry");
        writer.write_all(bytes).expect("zip bytes");
    }
    writer.finish().expect("finish zip");
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

struct Tools {
    actinglab: PathBuf,
    actingd: PathBuf,
    config: PathBuf,
}

impl Tools {
    fn actinglab(&self, args: &[&str]) -> (bool, Value) {
        let output = Command::new(&self.actinglab)
            .arg("--json")
            .args(args)
            .env("ACTINGLAB_CONFIG_PATH", &self.config)
            .output()
            .expect("run actinglab");
        let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "actinglab JSON: {error}\nstdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.success(), envelope)
    }

    fn digest(&self, dir: &Path) -> (bool, Value) {
        self.actinglab(&["package", "digest", "--package", dir.to_str().expect("utf-8 path")])
    }
}

/// The installer's own lay-out, every line it reports kept and logged.
fn lay(bundle: &Bundle, dir: &Path, log: &mut Log, label: &str) -> (Result<BTreeMap<String, PathBuf>, String>, Vec<String>) {
    let mut lines = Vec::new();
    let result = {
        let mut sink = |line: &str| -> Result<(), String> {
            lines.push(line.to_string());
            Ok(())
        };
        bundle.lay_out(dir, &mut sink)
    };
    for line in &lines {
        log.say(format!("{label} installer: {line}"));
    }
    match &result {
        Ok(laid) => log.say(format!("{label} lay_out Ok: {} packs", laid.len())),
        Err(reason) => log.say(format!("{label} lay_out Err: {}", reason.replace('\n', " | "))),
    }
    (result, lines)
}

/// Every laid-out directory is named by the digest `package digest` computes for it.
fn confirm(tools: &Tools, bundle: &Bundle, laid: &BTreeMap<String, PathBuf>, log: &mut Log, label: &str) {
    for pack in &bundle.packs {
        let path = &laid[&pack.path];
        let (ok, envelope) = tools.digest(path);
        let data = &envelope["data"];
        log.say(format!(
            "{label} {}: {} -> package digest ok={ok} reference={} package_id={} files={} bytes={}",
            pack.package_id,
            path.display(),
            data["reference"],
            data["package_id"],
            data["file_count"],
            data["byte_count"]
        ));
        assert!(ok, "{envelope}");
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some(pack.sha256.as_str()));
        assert_eq!(data["reference"]["schema_version"], CONTENT_DIRECTORY_V1);
        assert_eq!(data["reference"]["sha256"], pack.sha256.as_str());
        assert_eq!(data["package_id"], pack.package_id.as_str());
        assert_eq!(data["file_count"], pack.file_count);
        assert_eq!(data["byte_count"], pack.byte_count);
    }
}

#[test]
fn one_off_a3_bundle_v2_install_and_v1_unchanged() {
    let work = env_path("ONE_OFF_WORK");
    let v1_path = env_path("ONE_OFF_BUNDLE_V1");
    let tools = Tools {
        actinglab: env_path("ONE_OFF_ACTINGLAB"),
        actingd: env_path("ONE_OFF_ACTINGD"),
        config: work.join("actinglab-config.json"),
    };
    let mut log = Log(fs::File::create(work.join("one-off-a3.log")).expect("create the one-off log"));

    // The sealed v1 bundle of the public umbrella, unsealed as RT A2b's E3 did.
    let v1_bytes = fs::read(&v1_path).expect("read the v1 bundle");
    let v1_sha = hex(&Sha256::digest(&v1_bytes));
    log.say(format!("v1 bundle {} bytes sha256 {v1_sha} (expected {V1_SHA256})", v1_bytes.len()));
    assert_eq!(v1_sha, V1_SHA256);
    let outer = read_zip(&v1_bytes);
    let index: Value = serde_json::from_slice(&outer["bundle.json"]).expect("bundle v1 index");
    let applications = outer["applications.json"].clone();
    log.say(format!(
        "v1 index {} source_sha {} packs {} default_packs {}",
        index["schema_version"],
        index["source_sha"],
        index["packs"].as_array().expect("packs").len(),
        index["default_packs"]
    ));
    log.say(format!("applications.json: {}", String::from_utf8_lossy(&applications).replace('\n', " ")));
    let unsealed_root = work.join("unsealed");
    let admitted_root = work.join("admitted");
    let mut admitted = 0;
    for pack in index["packs"].as_array().expect("packs") {
        let package_id = pack["package_id"].as_str().expect("package id");
        let entries = read_zip(&outer[pack["path"].as_str().expect("path")]);
        let control: Value = serde_json::from_slice(&entries["control.json"]).expect("control");
        let stem = format!("{}.{}", control["game"].as_str().expect("game"), control["server"].as_str().expect("server"));
        let derived = [
            "resources/manifest.json".to_owned(),
            format!("resources/recognition/{stem}.pack.json"),
            format!("resources/recognition/{stem}.pages.json"),
            format!("resources/navigation/{stem}.navigation.json"),
            "resources/operations/operations.index.json".to_owned(),
            "resources/operations/operations.primitives.json".to_owned(),
        ];
        let mut unsealed = entries.clone();
        let removed = derived.iter().filter(|path| unsealed.remove(*path).is_some()).count();
        let directory = unsealed_root.join(package_id);
        write_tree(&directory, &unsealed);
        let (ok, envelope) = tools.digest(&directory);
        log.say(format!(
            "unseal {package_id}: sealed {} entries, removed {removed} derived, {} files; package digest ok={ok} {}",
            entries.len(),
            unsealed.len(),
            if ok { envelope["data"]["reference"].to_string() } else { envelope["error"].to_string() }
        ));
        if ok {
            write_tree(&admitted_root.join(package_id), &unsealed);
            admitted += 1;
        }
    }
    log.say(format!("{admitted} of {} unsealed packs admitted", index["packs"].as_array().expect("packs").len()));

    // `actinglab package bundle` over the admitted directories, zipped at the output's root.
    let applications_path = work.join("applications.json");
    fs::write(&applications_path, &applications).expect("write applications");
    let section = work.join("section");
    let (ok, envelope) = tools.actinglab(&[
        "package",
        "bundle",
        "--applications",
        applications_path.to_str().expect("path"),
        "--packs-root",
        admitted_root.to_str().expect("path"),
        "--out",
        section.to_str().expect("path"),
        "--source-repository",
        SOURCE_REPOSITORY,
        "--source-commit",
        index["source_sha"].as_str().expect("source sha"),
    ]);
    log.say(format!("package bundle ok={ok} error={}", envelope["error"]));
    assert!(ok, "{envelope}");
    log.say(format!("section top level {:?}", names(&section)));
    log.say(format!(
        "section bundle.json: {}",
        fs::read_to_string(section.join("bundle.json")).expect("bundle.json").replace('\n', " ")
    ));
    let section_files = read_tree(&section);
    let v2_path = work.join("bluearchive-bundle-v2.zip");
    write_zip(&v2_path, &section_files);
    log.say(format!(
        "v2 zip {}: {} entries, {} bytes, sha256 {}",
        v2_path.display(),
        section_files.len(),
        fs::metadata(&v2_path).expect("zip").len(),
        sha256_file(&v2_path).expect("hash")
    ));

    // The installer reads it as a bundle v2: packs, digests, the default from applications.json.
    let v2 = bundle::local(v2_path.to_str().expect("path")).expect("acsetup reads the v2 bundle");
    log.say(format!("acsetup read v2: game {} label {:?} packs {} defaults {:?}", v2.game, v2.label, v2.packs.len(), v2.defaults));
    for pack in &v2.packs {
        log.say(format!(
            "  pack {} server {} path {} digest {} files {} bytes {}",
            pack.package_id, pack.server, pack.path, pack.sha256, pack.file_count, pack.byte_count
        ));
    }
    assert_eq!(v2.packs.len(), admitted);
    let default_path = v2.defaults.get("jp").expect("jp default").clone();
    let default = v2.packs.iter().find(|pack| pack.path == default_path).expect("default listed").clone();
    log.say(format!("default jp -> {} ({})", default.package_id, default.path));

    // 1. A clean root: every pack laid out as `<digest>\`, confirmed by `package digest`.
    let root = work.join("root");
    let dir = root.join("packages").join(&v2.game);
    let (result, _) = lay(&v2, &dir, &mut log, "clean");
    let laid = result.expect("clean lay out");
    assert_eq!(laid.len(), v2.packs.len());
    confirm(&tools, &v2, &laid, &mut log, "clean");
    log.say(format!("clean {}: {:?}", dir.display(), names(&dir)));
    assert!(names(&dir).iter().all(|name| name.len() == 64), "only digest-named directories");

    // 2. Again on the same root: every directory verifies and is reused.
    let (result, lines) = lay(&v2, &dir, &mut log, "again");
    assert_eq!(result.expect("second lay out"), laid);
    assert_eq!(lines.iter().filter(|line| line.contains("reused")).count(), v2.packs.len());

    // 3. A JSON value changed in the default pack's directory (E4 a): the loader refuses it,
    //    and the installer sets it aside as `<digest>.broken-<unix>`, keeps it and unpacks again.
    let target = laid[&default.path].clone();
    let control_path = target.join("control.json");
    let mut control: Value = serde_json::from_slice(&fs::read(&control_path).expect("control")).expect("control json");
    let key = ["timeout_ms", "max_steps", "step_timeout_ms", "capture_interval_ms"]
        .into_iter()
        .find(|key| control[*key].is_u64())
        .expect("a numeric value in control.json");
    let before = control[key].clone();
    control[key] = json!(control[key].as_u64().expect("numeric") + 1);
    fs::write(&control_path, serde_json::to_vec_pretty(&control).expect("json")).expect("rewrite control.json");
    log.say(format!("corrupted {}: {key} {before} -> {}", control_path.display(), control[key]));
    let (ok, envelope) = tools.digest(&target);
    log.say(format!("package digest on the corrupted directory: ok={ok} {}", envelope["error"]));
    assert!(!ok);
    let (result, lines) = lay(&v2, &dir, &mut log, "corrupted");
    let relaid = result.expect("lay out after the corruption");
    let broken: Vec<String> = names(&dir).into_iter().filter(|name| name.starts_with(&format!("{}.broken-", default.sha256))).collect();
    log.say(format!("after the corruption {}: {:?}", dir.display(), names(&dir)));
    assert_eq!(broken.len(), 1, "one directory set aside");
    assert!(lines.iter().any(|line| line.contains(&broken[0])), "the set-aside is said");
    let kept: Value = serde_json::from_slice(&fs::read(dir.join(&broken[0]).join("control.json")).expect("kept control")).expect("json");
    log.say(format!("kept {}: control.json {key} {}", broken[0], kept[key]));
    assert_eq!(kept[key], control[key]);
    confirm(&tools, &v2, &relaid, &mut log, "corrupted");

    // 4. A file removed (E4 b, an interrupted copy) together with a leftover `.part`: both set
    //    aside — the second under `-2` when the second is the same — and the pack unpacked again.
    let other = v2.packs.iter().find(|pack| pack.path != default.path).unwrap_or(&default).clone();
    let other_dir = relaid[&other.path].clone();
    let other_files = read_tree(&other_dir);
    let (removed_name, _) = other_files.iter().rev().next().expect("a file");
    let removed_path = removed_name.split('/').fold(other_dir.clone(), |parent, part| parent.join(part));
    let parked = work.join("removed-file");
    fs::rename(&removed_path, &parked).expect("move one file out (kept in the work folder)");
    let leftover = dir.join(format!("{}.part", other.sha256));
    fs::create_dir_all(&leftover).expect("leftover part");
    fs::write(leftover.join("half.json"), b"{").expect("leftover file");
    log.say(format!("moved {removed_name} out of {} to {}; made {}", other_dir.display(), parked.display(), leftover.display()));
    let (result, lines) = lay(&v2, &dir, &mut log, "removed+part");
    let relaid = result.expect("lay out after the removal");
    log.say(format!("after the removal {}: {:?}", dir.display(), names(&dir)));
    assert_eq!(lines.iter().filter(|line| line.contains(".broken-")).count(), 2);
    confirm(&tools, &v2, &relaid, &mut log, "removed+part");

    // 5. A bundle whose pack bytes do not match their digest: the unpack fails loudly, and
    //    the unfinished directory is kept under `.broken-<unix>`; nothing is renamed into place.
    let mut tampered = section_files.clone();
    let tampered_name = format!("{}/control.json", default.path);
    tampered.get_mut(&tampered_name).expect("control.json").push(b' ');
    let tampered_zip = work.join("bluearchive-bundle-v2-tampered.zip");
    write_zip(&tampered_zip, &tampered);
    let tampered_bundle = bundle::local(tampered_zip.to_str().expect("path")).expect("the index itself is intact");
    let tampered_dir = work.join("root-tampered").join("packages").join(&tampered_bundle.game);
    let (result, _) = lay(&tampered_bundle, &tampered_dir, &mut log, "tampered");
    let reason = result.expect_err("a tampered pack is refused");
    log.say(format!("tampered {}: {:?}", tampered_dir.display(), names(&tampered_dir)));
    assert!(reason.contains(crate::verify::MISMATCH) && reason.contains(".broken-"), "{reason}");
    assert!(!tampered_dir.join(&default.sha256).exists());

    // 6. check-config admits the default pack's directory in full (a fixture instance).
    let state_root = work.join("state");
    fs::create_dir_all(&state_root).expect("state root");
    let config = work.join("actingd.config.json");
    let config_value = json!({
        "schema_version": "actingcommand.actingd.config.v1",
        "state_root": state_root,
        "bind_host": "127.0.0.1",
        "bind_port": 0,
        "secret_fingerprint_salt": "one-off-a3-fixture-salt",
        "instances": [{
            "alias": "one-off",
            "instance_id": format!("instance_{}", "0".repeat(31) + "1"),
            "fixture_backend": {"frames": [{"width": 1, "height": 1, "rgb": [1, 2, 3]}], "max_inputs": 1},
            "resource_package": relaid[&default.path]
        }]
    });
    fs::write(&config, serde_json::to_vec_pretty(&config_value).expect("json")).expect("config");
    let output = Command::new(&tools.actingd)
        .args(["check-config", "--config", config.to_str().expect("path")])
        .output()
        .expect("run check-config");
    let checked: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "check-config JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    log.say(format!(
        "check-config exit={} status={} resource_package={} not_checked={} error={}",
        output.status,
        checked["status"],
        checked["instances"][0]["resource_package"],
        checked["not_checked"],
        checked["error"]
    ));
    assert!(output.status.success(), "{checked}");

    // 7. The v1 bundle installs as before: a clean root gets the sealed zips byte for byte;
    //    laid out next to the v2 directories, both stay, and the v2 directories are reused.
    let v1 = bundle::local(v1_path.to_str().expect("path")).expect("acsetup reads the v1 bundle");
    log.say(format!("acsetup read v1: game {} packs {} defaults {:?}", v1.game, v1.packs.len(), v1.defaults));
    let v1_dir = work.join("root-v1").join("packages").join(&v1.game);
    let (result, _) = lay(&v1, &v1_dir, &mut log, "v1");
    let v1_laid = result.expect("v1 lay out");
    for pack in &v1.packs {
        let path = &v1_laid[&pack.path];
        let sha = sha256_file(path).expect("hash");
        log.say(format!("v1 {} -> {} sha256 {sha} (bundle.json {})", pack.package_id, path.display(), pack.sha256));
        assert_eq!(sha, pack.sha256);
        assert_eq!(fs::metadata(path).expect("zip").len(), pack.byte_count);
    }
    assert!(names(&v1_dir).iter().all(|name| name.ends_with(".zip")));
    let (result, _) = lay(&v1, &dir, &mut log, "v1-beside-v2");
    result.expect("v1 beside v2");
    let (result, lines) = lay(&v2, &dir, &mut log, "v2-after-v1");
    result.expect("v2 after v1");
    assert_eq!(lines.iter().filter(|line| line.contains("reused")).count(), v2.packs.len());
    log.say(format!("final {}: {:?}", dir.display(), names(&dir)));
    log.say("all checks passed");
}
