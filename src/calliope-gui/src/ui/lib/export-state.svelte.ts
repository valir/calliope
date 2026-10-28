import {
  cancelExport,
  chooseRc5Folder,
  exportInfo,
  frontendLog,
  prepareExport,
  startExport,
  watchExport,
  type ExportEvent,
  type ExportInfo,
  type ExportKind,
  type ExportPlan,
  type ExportSnapshot,
  type ExportTargetReq,
  type TrackRecord,
} from './ipc';
import { mergeTracks } from './library-state.svelte';

export const ex = $state({
  info: null as ExportInfo | null,
  /** The Memory box (RC-5 export). */
  memory: '',
  snap: null as ExportSnapshot | null,
  /** A refusal or start error, shown like a failure. */
  message: '',
  /** Waiting for the folder dialog. */
  preparing: false,
  /** The overwrite confirmation, while it is open. */
  confirm: null as { plan: ExportPlan; target: ExportKind } | null,
  /** The folder the next RC-5 exports use without the dialog (the "To:" line); null = ask. */
  rc5Quick: null as string | null,
  /** A "Change..." refusal, shown on the result line. */
  notice: '',
});

let infoSeq = 0;
/** The track the Memory box was last filled for. */
let filledFor: string | null = null;
/** A refill asked for by a load that lost the `infoSeq` race. */
let refillPending = false;
/** The track the pane currently shows (the last id given to `loadInfo`). */
let targetId: string | null = null;
/** Events that arrived before the command's snapshot did. */
let pending: ExportEvent[] = [];
/** The base folder of the RC-5 plan being started (becomes `ex.rc5Quick` when its job is done). */
let startedBase: string | null = null;

function setQuick(v: string | null): void {
  if (ex.rc5Quick === v) return;
  ex.rc5Quick = v;
  void frontendLog(`export quick dir=${v ?? 'none'}`);
}

/** A new visit of the Export view: nothing is remembered from an earlier one. */
export function beginExportVisit(): void {
  setQuick(null);
  startedBase = null;
  ex.notice = '';
}

export function resetExport(): void {
  ex.info = null;
  ex.memory = '';
  ex.snap = null;
  ex.message = '';
  ex.preparing = false;
  ex.confirm = null;
  ex.rc5Quick = null;
  ex.notice = '';
  startedBase = null;
  infoSeq = 0;
  filledFor = null;
  refillPending = false;
  targetId = null;
  pending = [];
}

export function isRunning(): boolean {
  return ex.snap?.phase === 'writing' || ex.snap?.phase === 'finishing';
}

/** Loads the export info of a track (null clears it). `refill` replaces the Memory box text. */
export async function loadInfo(id: string | null, refill = false): Promise<void> {
  const seq = ++infoSeq;
  targetId = id;
  if (refill && id !== null) refillPending = true;
  if (id === null) {
    ex.info = null;
    return;
  }
  if (ex.info && ex.info.id !== id) ex.info = null;
  try {
    const info = await exportInfo(id);
    if (seq !== infoSeq) return;
    ex.info = info;
    if (refillPending || ex.memory === '' || filledFor !== id) {
      ex.memory = String(info.memory);
      filledFor = id;
    }
    refillPending = false;
    const l = info.last;
    void frontendLog(
      `export last id=${id} memory=${ex.memory} rc5=${l?.rc5 ? l.rc5.memory : 'none'} ` +
        `mp3=${l?.mp3 ? l.mp3.path : 'none'} ogg=${l?.ogg ? l.ogg.path : 'none'}`,
    );
  } catch (e) {
    if (seq !== infoSeq) return;
    ex.info = null;
    ex.message = String(e);
  }
}

export function applyEvent(e: ExportEvent): void {
  const s = ex.snap;
  if (!s) {
    pending.push(e);
    return;
  }
  void frontendLog(`export phase=${e.phase}`);
  s.phase = e.phase;
  if (e.phase === 'writing') s.progress = e.progress;
  else if (e.phase === 'done') {
    s.path = e.path;
    s.clipped = e.clipped;
    s.progress = 1;
    s.info_error = e.info_error ?? null;
    void frontendLog(`export done path=${e.path}`);
    mergeTracks(e.root, e.records ?? []);
    // Only the track the pane shows needs its info reloaded; another one is loaded when selected.
    if (targetId === s.id) void loadInfo(s.id, true);
    if (s.target === 'rc5' && startedBase !== null) setQuick(startedBase);
  } else if (e.phase === 'failed') {
    s.message = e.message;
    void frontendLog(`export failed message=${e.message}`);
    if (s.target === 'rc5') setQuick(null);
  } else if (e.phase === 'cancelled' && s.target === 'rc5') {
    setQuick(null);
  }
  if ((e.phase === 'done' || e.phase === 'failed' || e.phase === 'cancelled') && s.target === 'rc5') {
    startedBase = null;
  }
  if (e.phase === 'done' && e.info_error) void frontendLog(`export info-error message=${e.info_error}`);
}

function takeSnapshot(snap: ExportSnapshot): void {
  // Events that arrived while the command was running are newer than its answer.
  ex.snap = snap;
  const early = pending;
  pending = [];
  for (const e of early) applyEvent(e);
}

async function start(plan: ExportPlan, kind: ExportKind): Promise<void> {
  ex.message = '';
  startedBase = kind === 'rc5' ? plan.base : null;
  ex.snap = null;
  pending = [];
  try {
    const snap = await startExport(plan.token, plan.exists, applyEvent);
    takeSnapshot(snap);
  } catch (e) {
    ex.message = String(e);
    void frontendLog(`export failed message=${ex.message}`);
    if (kind === 'rc5') {
      setQuick(null);
      startedBase = null;
    }
  }
}

export async function exportTo(kind: ExportKind, track: TrackRecord | null): Promise<void> {
  if (!track || ex.preparing || isRunning()) return;
  // Same rule as the buttons: nothing is asked or sent unless the track's info says it can be exported.
  if (!ex.info || ex.info.id !== track.id || !ex.info.available) return;
  const target: ExportTargetReq = kind === 'rc5' ? { kind, memory: ex.memory.trim(), reuse: ex.rc5Quick } : { kind };
  ex.message = '';
  ex.notice = '';
  ex.preparing = true;
  let plan: ExportPlan | null;
  try {
    plan = await prepareExport(track.id, target);
  } catch (e) {
    ex.message = String(e);
    void frontendLog(`export prepare target=${kind} result=refused exists=false`);
    return;
  } finally {
    ex.preparing = false;
  }
  if (!plan) {
    void frontendLog(`export prepare target=${kind} result=cancelled exists=false`);
    // Only a dialog that was cancelled returns nothing: ask again next time.
    if (kind === 'rc5') setQuick(null);
    return;
  }
  void frontendLog(`export prepare target=${kind} result=plan exists=${plan.exists}`);
  if (plan.exists) {
    ex.confirm = { plan, target: kind };
    return;
  }
  await start(plan, kind);
}

export async function answerConfirm(yes: boolean): Promise<void> {
  const c = ex.confirm;
  ex.confirm = null;
  if (!c) return;
  void frontendLog(`export confirm answer=${yes ? 'yes' : 'no'} path=${c.plan.path}`);
  if (yes) await start(c.plan, c.target);
}

/** "Change...": picks the folder of the next RC-5 exports; exports nothing. */
export async function changeRc5Folder(): Promise<void> {
  if (isRunning() || ex.preparing) return;
  ex.preparing = true;
  ex.notice = '';
  ex.message = '';
  let result = 'cancelled';
  try {
    const r = await chooseRc5Folder();
    if (r !== null) {
      setQuick(r);
      result = 'picked';
    }
  } catch (e) {
    ex.notice = `Folder not changed: ${e}`;
    result = 'refused';
  } finally {
    void frontendLog(`export change result=${result}`);
    ex.preparing = false;
  }
}

export async function cancel(): Promise<void> {
  const s = ex.snap;
  if (!s || !isRunning()) return;
  try {
    const r = await cancelExport(s.job);
    if (r && ex.snap && ex.snap.job === r.job && ex.snap.phase !== 'done') ex.snap = r;
  } catch (e) {
    ex.message = String(e);
  }
}

/** Re-attaches to a running or finished export after a webview reload. */
export async function attachExport(): Promise<void> {
  try {
    const snap = await watchExport(applyEvent);
    if (snap) takeSnapshot(snap);
    else pending = [];
  } catch {
    // No backend: nothing to attach to.
  }
}
