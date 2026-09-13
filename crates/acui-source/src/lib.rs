// SPDX-License-Identifier: AGPL-3.0-only
//! Where rows come from. v0 has exactly one implementation: exported page files.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use acui_rows::{EventRow, OpenEnvelope, OpenReport, PageEnvelope};
use anyhow::{bail, Context, Result};

pub trait EventSource {
    fn rows(&self) -> &[EventRow];
    fn open_report(&self) -> Option<&OpenReport>;
    fn source_label(&self) -> String;
}

/// Rows merged from one or more exported `events` page files.
pub struct FileSource {
    rows: Vec<EventRow>,
    open: Option<OpenReport>,
    label: String,
}

impl FileSource {
    /// No file given: an honest empty console.
    pub fn empty() -> Self {
        Self {
            rows: Vec::new(),
            open: None,
            label: "未提供文件".to_string(),
        }
    }

    pub fn load(paths: &[PathBuf], open_path: Option<&Path>) -> Result<Self> {
        let mut rows = Vec::new();
        for path in paths {
            let file = File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
            let envelope: PageEnvelope = serde_json::from_reader(BufReader::new(file))
                .with_context(|| format!("解析 {} 失败", path.display()))?;
            if envelope.command != "events" {
                bail!("{} 不是 events 页（command={}）", path.display(), envelope.command);
            }
            rows.extend(envelope.data.events);
        }
        rows.sort_by_key(|row| row.sequence);
        rows.dedup_by_key(|row| row.sequence);

        let open = match open_path {
            Some(path) => {
                let file =
                    File::open(path).with_context(|| format!("打开 {} 失败", path.display()))?;
                let envelope: OpenEnvelope = serde_json::from_reader(BufReader::new(file))
                    .with_context(|| format!("解析 {} 失败", path.display()))?;
                if envelope.command != "open" {
                    bail!("{} 不是 open 报告（command={}）", path.display(), envelope.command);
                }
                Some(envelope.data)
            }
            None => None,
        };

        Ok(Self {
            rows,
            open,
            label: label_for(paths, open_path),
        })
    }

    /// Hand the loaded rows to the view model.
    pub fn into_parts(self) -> (Vec<EventRow>, Option<OpenReport>, String) {
        (self.rows, self.open, self.label)
    }
}

fn label_for(paths: &[PathBuf], open_path: Option<&Path>) -> String {
    let mut names: Vec<String> = paths
        .iter()
        .map(|path| file_name(path).to_string())
        .collect();
    if let Some(path) = open_path {
        names.push(file_name(path).to_string());
    }
    names.join(" + ")
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
}

impl EventSource for FileSource {
    fn rows(&self) -> &[EventRow] {
        &self.rows
    }

    fn open_report(&self) -> Option<&OpenReport> {
        self.open.as_ref()
    }

    fn source_label(&self) -> String {
        self.label.clone()
    }
}
