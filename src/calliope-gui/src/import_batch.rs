//! The directory import (plan section 2.5): one job in the import slot that runs a list of
//! local audio files one after the other, reusing the single-file code (`prepare_file`,
//! `extract_cleaned`) for each. A file's failure is recorded and the batch goes on; Cancel stops
//! the current file and skips the rest. Child module of `import_job` (it uses its private items).

use super::*;

impl ImportState {
    /// Starts importing `files` (absolute paths, in import order, all inside `dir`). Nothing
    /// starts on an error: no files, no edge-AI address, missing tools, a job already active,
    /// or repository problems.
    pub fn start_batch(
        &self,
        root: &Path,
        dir: &Path,
        files: Vec<PathBuf>,
        edge_ai_url: Option<&str>,
        keep_original: bool,
        sink: Sink,
    ) -> Result<JobSnapshot, String> {
        if files.is_empty() {
            return Err("There are no files to import".into());
        }
        if files.iter().any(|f| !f.is_absolute()) {
            return Err("the selected file path must be absolute".into());
        }
        let url = edge_ai_url.map(str::trim).filter(|u| !u.is_empty()).ok_or_else(|| NOT_CONFIGURED_MESSAGE.to_string())?;
        let url = url.to_string();
        check_tools(&self.inner.current_tools(), &[ToolName::Ffmpeg, ToolName::Ffprobe])?;
        let mut slot = guard(&self.inner.job);
        if slot.as_ref().is_some_and(|j| !j.over) {
            return Err(BUSY_MESSAGE.into());
        }
        check_root(root, &*self.inner.lock)?;
        let tmp = ImportTmp::open(root)?;
        tmp.clean_stale(now_secs(), None);
        locked(&*self.inner.lock, || Repository::new(root).clean_stale_staging(None));
        let names: Vec<String> = files.iter().map(|f| file_name_of(f)).collect();
        let label = dir.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| dir.display().to_string());
        let id = track_meta::new_id();
        let mut snap = new_snapshot(id.clone(), SourceKind::Directory, label, Phase::Preparing);
        snap.batch = Some(BatchSnapshot { dir: dir.display().to_string(), files: names.clone(), current: Some(0), items: Vec::new() });
        eprintln!("calliope: import batch job={id} files={}", files.len());
        self.launch(&mut slot, snap, SourceKind::Directory, PathBuf::new(), root, tmp, sink, move |ctx| {
            run_batch(&ctx, &files, &names, &url, keep_original);
        })
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn stage_name(stage: Stage) -> String {
    format!("{stage:?}").to_lowercase()
}

/// One file: its own temp folder (removed whatever happens), `prepare_file`, `extract_cleaned`.
fn run_one(ctx: &Ctx, path: &Path, name: &str, url: &str, keep_original: bool) -> R<(TrackRecord, Vec<DroppedStem>)> {
    let dir = match ctx.tmp.new_file_dir("audio-file", name, &track_meta::now_rfc3339()) {
        Ok(d) => d,
        Err(m) => return fail(Stage::Prepare, m),
    };
    ctx.update(|j| j.dir = dir.clone());
    let r = (|| {
        let p = prepare_file(ctx, &dir, path, name)?;
        ctx.update(|j| {
            j.snap.metadata = Some(p.edits.clone());
            j.snap.duration_s = p.duration_s;
        });
        ctx.phase(Phase::Uploading, ImportEvent::Uploading { sent: 0, total: 0 });
        extract_cleaned(ctx, &dir, &p.audio, &p.edits, url, keep_original, p.position)
    })();
    if r.is_err() && dir.exists() {
        if let Err(e) = ctx.tmp.remove_job_dir(&dir) {
            eprintln!("calliope: import cleanup: {e}");
        }
    }
    r
}

fn run_batch(ctx: &Ctx, files: &[PathBuf], names: &[String], url: &str, keep_original: bool) {
    let n = files.len();
    let mut cancelled = false;
    for (i, path) in files.iter().enumerate() {
        if ctx.cancel.is_cancelled() {
            cancelled = true;
            break;
        }
        let name = &names[i];
        ctx.update(|j| {
            let s = &mut j.snap;
            s.downloaded = 0;
            s.total = None;
            s.sent = 0;
            s.progress = None;
            s.stems_done = 0;
            s.stems_total = 0;
            s.dropped.clear();
            s.metadata = None;
            s.duration_s = None;
            s.error = None;
            if let Some(b) = s.batch.as_mut() {
                b.current = Some(i);
            }
        });
        eprintln!("calliope: import batch job={} file={}/{n} name={name} result=started", ctx.id, i + 1);
        ctx.emit(ImportEvent::BatchFile { index: i, total: n, name: name.clone() });
        ctx.phase(Phase::Preparing, ImportEvent::Preparing);
        let outcome = match run_one(ctx, path, name, url, keep_original) {
            Ok((track, dropped)) => {
                let d = if dropped.is_empty() { "none".to_string() } else { dropped.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(",") };
                eprintln!("calliope: import batch job={} file={}/{n} name={name} result=saved id={} dropped={d}", ctx.id, i + 1, track.id);
                BatchOutcome::Saved { track, dropped }
            }
            Err(Stop::Failed(e)) => {
                eprintln!(
                    "calliope: import batch job={} file={}/{n} name={name} result=failed stage={} message={}",
                    ctx.id,
                    i + 1,
                    stage_name(e.stage),
                    e.message
                );
                BatchOutcome::Failed { stage: e.stage, message: e.message }
            }
            Err(Stop::Cancelled) => {
                eprintln!("calliope: import batch job={} file={}/{n} name={name} result=cancelled", ctx.id, i + 1);
                cancelled = true;
                break;
            }
        };
        let item = BatchItem { index: i, name: name.clone(), outcome };
        ctx.update(|j| {
            if let Some(b) = j.snap.batch.as_mut() {
                b.items.push(item.clone());
            }
        });
        ctx.emit(ImportEvent::BatchItem { item });
    }
    finish_batch(ctx, cancelled);
}

fn finish_batch(ctx: &Ctx, cancelled: bool) {
    let phase = if cancelled { Phase::Cancelled } else { Phase::Finished };
    let mut batch: Option<BatchSnapshot> = None;
    ctx.update(|j| {
        j.snap.phase = phase;
        if let Some(b) = j.snap.batch.as_mut() {
            b.current = None;
            batch = Some(b.clone());
        }
        j.running = false;
        j.over = true;
        j.cancel = None;
        j.audio = None;
    });
    let Some(batch) = batch else { return };
    let saved = batch.items.iter().filter(|i| matches!(i.outcome, BatchOutcome::Saved { .. })).count();
    eprintln!("calliope: import job={} phase={phase:?}", ctx.id);
    if cancelled {
        eprintln!("calliope: import batch job={} cancelled saved={saved} skipped={}", ctx.id, batch.files.len() - batch.items.len());
        ctx.emit(ImportEvent::Cancelled { back_to: BackTo::Source });
    } else {
        eprintln!("calliope: import batch job={} finished saved={saved} failed={}", ctx.id, batch.items.len() - saved);
        ctx.emit(ImportEvent::Finished { batch });
    }
}
