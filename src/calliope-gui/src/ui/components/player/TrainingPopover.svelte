<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import * as Popover from '$lib/components/ui/popover/index.js';
  import { Switch } from '$lib/components/ui/switch/index.js';
  import { parseTrainingField } from '$lib/speed-format';
  import type { PlayerLoop, TrainingPlan } from '$lib/ipc';
  import { frontendLog } from '$lib/ipc';
  import { pl, setTraining } from '$lib/player-state.svelte';

  const snap = $derived(pl.snap);
  const active = $derived(pl.status === 'active' && snap !== null);
  const cur = $derived<PlayerLoop | null>(snap && snap.current_loop !== null ? (snap.loops[snap.current_loop] ?? null) : null);
  const complete = $derived(cur !== null && cur.end !== null);
  const plan = $derived<TrainingPlan>(cur?.training ?? { on: false, start: 70, end: 100, step: 5, repeats: 2 });
  const on = $derived(complete && plan.on);
  const pressed = 'bg-amber-500! text-black! hover:bg-amber-500/90!';

  type Field = 'start' | 'end' | 'step' | 'repeats';
  const FIELDS: { key: Field; label: string; min: number; max: number; message: string }[] = [
    { key: 'start', label: 'Start %', min: 25, max: 200, message: 'Training speeds must be between 25 and 200 %' },
    { key: 'end', label: 'End %', min: 25, max: 200, message: 'Training speeds must be between 25 and 200 %' },
    { key: 'step', label: 'Step %', min: 1, max: 175, message: 'The step must be between 1 and 175 %' },
    { key: 'repeats', label: 'Repeats', min: 1, max: 99, message: 'Repeats must be between 1 and 99' },
  ];

  const sign = (p: TrainingPlan): string => (p.end < p.start ? '−' : '+');
  const title = $derived(
    !active ? 'Open a track first'
    : !complete ? 'Set the loop end first'
    : on ? `Speed training on: ${plan.start} → ${plan.end} %, ${sign(plan)}${plan.step} %, ×${plan.repeats}`
    : 'Speed training settings',
  );
  const summary = $derived(
    `${plan.start} → ${plan.end} %, ${sign(plan)}${plan.step} % every ${plan.repeats} ${plan.repeats === 1 ? 'pass' : 'passes'}`,
  );
  const loopText = $derived(snap && cur ? `Loop ${(snap.current_loop ?? 0) + 1} of ${snap.loops.length}` : '');

  let isOpen = $state(false);
  let texts = $state<Record<Field, string | null>>({ start: null, end: null, step: null, repeats: null });
  let invalid = $state<Record<Field, boolean>>({ start: false, end: false, step: false, repeats: false });
  let message = $state('');
  let switchRef = $state<HTMLElement | null>(null);
  let contentRef = $state<HTMLElement | null>(null);
  let triggerRef = $state<HTMLElement | null>(null);

  const shown = (f: Field): string => texts[f] ?? String(plan[f]);

  function clearEdits(): void {
    texts = { start: null, end: null, step: null, repeats: null };
    invalid = { start: false, end: false, step: false, repeats: false };
    message = '';
  }
  function commit(f: Field): void {
    const t = texts[f];
    if (t === null) return;
    const spec = FIELDS.find((x) => x.key === f)!;
    const n = parseTrainingField(t, spec.min, spec.max);
    if (n === null) {
      invalid[f] = true;
      message = spec.message;
      return;
    }
    texts[f] = null;
    invalid[f] = false;
    message = '';
    void setTraining({ ...plan, [f]: n });
  }
  function revert(f: Field): void {
    texts[f] = null;
    invalid[f] = false;
    message = '';
  }
  function keydown(f: Field, e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commit(f);
    } else if (e.key === 'Escape') {
      // Escape closes the whole popover (below); the edit is dropped with it.
      revert(f);
    }
  }
  function blur(f: Field): void {
    if (texts[f] === null) return;
    if (!invalid[f]) commit(f);
  }

  function close(via: 'escape' | 'outside' | 'button' | 'tab'): void {
    if (!isOpen) return;
    if (via === 'escape') clearEdits();
    else {
      for (const f of FIELDS) commit(f.key);
      clearEdits();
    }
    isOpen = false;
    void frontendLog(`player training-popover close via=${via}`);
  }
  function openChange(v: boolean): void {
    if (v) {
      if (!complete) return;
      clearEdits();
      isOpen = true;
    } else close('button');
  }
  let placedFor = 0;
  function openAutoFocus(e: Event): void {
    e.preventDefault();
    const mine = ++placedFor;
    switchRef?.focus();
    // floating-ui places the content after a frame or two; it is not focusable before that
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (isOpen && !contentRef?.contains(document.activeElement)) switchRef?.focus();
      const r = contentRef?.getBoundingClientRect();
      if (!r || !isOpen || mine !== placedFor) return;
      const n = (v: number): number => Math.round(v);
      void frontendLog(
        `player training-popover open loop=${snap?.current_loop ?? 'none'} rect=${n(r.left)},${n(r.top)},${n(r.right)},${n(r.bottom)} viewport=${window.innerWidth}x${window.innerHeight}`,
      );
    }));
  }
  function closeKey(e: KeyboardEvent): void {
    if (e.key === 'Tab' && !e.shiftKey) {
      e.preventDefault();
      close('tab');
    }
  }
  $effect(() => {
    if (isOpen && !complete) close('button');
  });
  const box =
    'h-11 w-full rounded-md border border-input bg-background px-2 text-center text-lg tabular-nums aria-invalid:border-destructive';
</script>

<Popover.Root open={isOpen} onOpenChange={openChange}>
  <Popover.Trigger bind:ref={triggerRef}>
    {#snippet child({ props })}
      <Button
        {...props}
        variant="outline"
        class="h-[3.25rem] px-2 text-lg {on ? pressed : ''}"
        aria-label="Speed training settings"
        aria-pressed={on}
        disabled={!active || !complete}
        {title}
      >Train…</Button>
    {/snippet}
  </Popover.Trigger>
  <Popover.Content
    bind:ref={contentRef}
    side="left"
    align="end"
    sideOffset={8}
    collisionPadding={8}
    trapFocus={false}
    class="w-80 p-3"
    role="dialog"
    aria-label="Speed training"
    onOpenAutoFocus={openAutoFocus}
    onCloseAutoFocus={(e) => { e.preventDefault(); triggerRef?.focus(); }}
    onEscapeKeydown={() => close('escape')}
    onInteractOutside={() => close('outside')}
  >
    <div class="flex flex-col gap-3">
      <div class="flex items-baseline justify-between gap-2">
        <span class="text-lg font-semibold">Speed training</span>
        <span class="text-base text-muted-foreground">{loopText}</span>
      </div>
      <label class="flex items-center gap-3 text-lg">
        <Switch bind:ref={switchRef} checked={on} aria-label="Training on" onCheckedChange={(v) => void setTraining({ ...plan, on: v })} />
        Training on
      </label>
      <div class="grid grid-cols-2 gap-x-3 gap-y-2">
        {#each FIELDS as f (f.key)}
          <label class="flex flex-col gap-1 text-base">
            {f.label}
            <input
              type="text" inputmode="numeric" class={box} aria-label={f.label}
              aria-invalid={invalid[f.key] ? 'true' : undefined} value={shown(f.key)}
              oninput={(e) => { texts[f.key] = e.currentTarget.value; invalid[f.key] = false; message = ''; }}
              onkeydown={(e) => keydown(f.key, e)} onblur={() => blur(f.key)}
            />
          </label>
        {/each}
      </div>
      <div class="text-base">{summary}</div>
      <div class="min-h-5 text-base text-destructive" role="status" data-testid="training-message">{message}</div>
      <Button variant="outline" class="h-11 self-end px-4 text-lg" onclick={() => close('button')} onkeydown={closeKey}>Close</Button>
    </div>
  </Popover.Content>
</Popover.Root>
