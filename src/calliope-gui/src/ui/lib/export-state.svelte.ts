import {
  cancelExport,
  chooseRc5Folder,
  chooseRc500Folder,
  exportInfo,
  frontendLog,
  prepareExport,
  setSlowCountIn as saveSlowCountIn,
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
  /** The RC-500 Memory box text and its Track choice. */
  rc500Memory: '',
  rc500Track: 1 as 1 | 2,
  snap: null as ExportSnapshot | null,
  /** A refusal or start error, shown like a failure. */
  message: '',
  /** Waiting for the folder dialog. */
  preparing: false,
  /** The overwrite confirmation, while it is open. */
  confirm: null as { plan: ExportPlan; target: ExportKind } | null,
  /** The folder the next RC-5 exports use without the dialog (the "To:" line); null = ask. */
  rc5Quick: null as string | null,
  /** The same for the RC-500. */
  rc500Quick: null as string | null,
  /** A "Change..." refusal, shown on the result line. */
  notice: '',
  /** The slow count-in switch is being written. */
  slowSaving: false,
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
/** The kind and base folder of the looper plan being started (becomes that kind's quick folder when done). */
let startedBase: { kind: 'rc5' | 'rc500'; base: string } | null = null;

type LooperKind = 'rc5' | 'rc500';
const isLooper = (k: ExportKind): k is LooperKind => k === 'rc5' || k === 'rc500';

function setQuick(kind: LooperKind, v: string | null): void {
  if (kind === 'rc5') {
    if (ex.rc5Quick === v) return;
    ex.rc5Quick = v;
    void frontendLog(`export quick dir=${v ?? 'none'}`);
  } else {
    if (ex.rc500Quick === v) return;
    ex.rc500Quick = v;
    void frontendLog(`export quick kind=rc500 dir=${v ?? 'none'}`);
  }
}

/** A new visit of the Export view: nothing is remembered from an earlier one. */
export function beginExportVisit(): void {
  setQuick('rc5', null);
  setQuick('rc500', null);
  startedBase = null;
  ex.notice = '';
}

export function resetExport(): void {
  ex.info = null;
  ex.memory = '';
  ex.rc500Memory = '';
  ex.rc500Track = 1;
  ex.snap = null;
  ex.message = '';
  ex.preparing = false;
  ex.confirm = null;
  ex.rc5Quick = null;
  ex.rc500Quick = null;
  ex.notice = '';
  ex.slowSaving = false;
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
    if (refillPending || filledFor !== id) {
      ex.memory = String(info.memory);
      ex.rc500Memory = String(info.rc500_memory);
      ex.rc500Track = info.rc500_track === 2 ? 2 : 1;
      filledFor = id;
    } else {
      if (ex.memory === '') ex.memory = String(info.memory);
      if (ex.rc500Memory === '') {
        ex.rc500Memory = String(info.rc500_memory);
        ex.rc500Track = info.rc500_track === 2 ? 2 : 1;
      }
    }
    refillPending = false;
    const l = info.last;
    void frontendLog(
      `export last id=${id} memory=${ex.memory} rc5=${l?.rc5 ? l.rc5.memory : 'none'} ` +
        `mp3=${l?.mp3 ? l.mp3.path : 'none'} ogg=${l?.ogg ? l.ogg.path : 'none'}`,
    );
    const r5 = l?.rc500;
    void frontendLog(
      `export last-rc500 id=${id} memory=${ex.rc500Memory} track=${ex.rc500Track} ` +
        `recorded=${r5 ? `${r5.memory}/${r5.track}` : 'none'}`,
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
    if (isLooper(s.target) && startedBase !== null && startedBase.kind === s.target) {
      setQuick(s.target, startedBase.base);
    }
  } else if (e.phase === 'failed') {
    s.message = e.message;
    void frontendLog(`export failed message=${e.message}`);
    if (isLooper(s.target)) setQuick(s.target, null);
  } else if (e.phase === 'cancelled' && isLooper(s.target)) {
    setQuick(s.target, null);
  }
  if ((e.phase === 'done' || e.phase === 'failed' || e.phase === 'cancelled') && isLooper(s.target)) {
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
  startedBase = isLooper(kind) ? { kind, base: plan.base } : null;
  ex.snap = null;
  pending = [];
  try {
    const snap = await startExport(plan.token, plan.exists, applyEvent);
    takeSnapshot(snap);
  } catch (e) {
    ex.message = String(e);
    void frontendLog(`export failed message=${ex.message}`);
    if (isLooper(kind)) {
      setQuick(kind, null);
      startedBase = null;
    }
  }
}

export async function exportTo(kind: ExportKind, track: TrackRecord | null): Promise<void> {
  if (!track || ex.preparing || ex.slowSaving || isRunning()) return;
  // Same rule as the buttons: nothing is asked or sent unless the track's info says it can be exported.
  if (!ex.info || ex.info.id !== track.id || !ex.info.available) return;
  let target: ExportTargetReq;
  if (kind === 'rc5') target = { kind, memory: ex.memory.trim(), reuse: ex.rc5Quick };
  else if (kind === 'rc500') {
    target = { kind, memory: ex.rc500Memory.trim(), track: ex.rc500Track, reuse: ex.rc500Quick };
  } else target = { kind };
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
    if (isLooper(kind)) setQuick(kind, null);
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
      setQuick('rc5', r);
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

/** "Change..." of the RC-500 row: picks the folder of the next RC-500 exports; exports nothing. */
export async function changeRc500Folder(): Promise<void> {
  if (isRunning() || ex.preparing) return;
  ex.preparing = true;
  ex.notice = '';
  ex.message = '';
  let result = 'cancelled';
  try {
    const r = await chooseRc500Folder();
    if (r !== null) {
      setQuick('rc500', r);
      result = 'picked';
    }
  } catch (e) {
    ex.notice = `Folder not changed: ${e}`;
    result = 'refused';
  } finally {
    void frontendLog(`export change kind=rc500 result=${result}`);
    ex.preparing = false;
  }
}

/** The slow count-in switch of the shown track: written at once, the info answer replaces `ex.info`. */
export async function setSlowCountIn(on: boolean): Promise<void> {
  if (!ex.info || ex.slowSaving || ex.preparing || isRunning() || ex.confirm) return;
  const id = ex.info.id;
  ex.slowSaving = true;
  ex.message = '';
  let result = 'failed';
  try {
    const info = await saveSlowCountIn(id, on);
    ++infoSeq;
    if (targetId === id) ex.info = info;
    result = 'saved';
  } catch (e) {
    ex.message = String(e);
  } finally {
    void frontendLog(`export slow id=${id} on=${on} result=${result}`);
    ex.slowSaving = false;
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
