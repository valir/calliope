<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatOffset, offsetTime } from '$lib/section-meta';
  import { sectionOffset } from '$lib/waveform-geometry';
  import { beginOffsetEdit, cancelOffsetEdit, commitSectionOffset, findStart, pl } from '$lib/player-state.svelte';

  /** The section the row shows (the captured one while the field has the focus); null without sections. */
  let { target }: { target: number | null } = $props();

  const NEED_BPM = 'Type a BPM first';
  const segs = $derived(pl.snap?.tempo.segments ?? []);
  const hasTempo = $derived(pl.status === 'active' && target !== null && segs.length > 0 && segs[0].bpm > 0);
  const offset = $derived(hasTempo && target !== null && segs[target] ? sectionOffset(segs, target) : { steps: 0, ms: 0 });
  /** The text being typed; null while the field shows the stored offset. */
  let text = $state<string | null>(null);
  /** The last Enter was refused: the message in the pane comes from this field. */
  let invalid = $state(false);

  const fieldTitle = $derived(
    !hasTempo || target === null
      ? NEED_BPM
      : target === 0
        ? '1/64 beats from the found start'
        : segs[target].detected_ms != null
          ? '1/64 beats from the detected position'
          : "1/64 beats from the previous section's start",
  );
  const noFind = $derived(!hasTempo || target !== 0 || pl.findingStart);
  const findTitle = $derived(
    !hasTempo ? NEED_BPM : pl.findingStart ? 'Finding the start...' : target !== 0 ? 'Find start works in the first section' : undefined,
  );

  function onfocus(): void {
    beginOffsetEdit();
    text = null;
    invalid = false;
  }

  function oninput(e: Event & { currentTarget: HTMLInputElement }): void {
    text = e.currentTarget.value;
    invalid = false;
  }

  async function onkeydown(e: KeyboardEvent & { currentTarget: HTMLInputElement }): Promise<void> {
    const el = e.currentTarget;
    if (e.key === 'Enter') {
      e.preventDefault();
      if (text === null) {
        el.blur();
        return;
      }
      if (await commitSectionOffset(text)) {
        text = null;
        invalid = false;
        el.blur();
      } else {
        invalid = true;
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      text = null;
      if (invalid) pl.sectionError = '';
      invalid = false;
      cancelOffsetEdit();
      el.blur();
    }
  }

  async function onblur(): Promise<void> {
    const t = text;
    const refused = invalid;
    text = null;
    invalid = false;
    // A refused Enter already showed its message: blurring cancels the edit instead of committing again.
    if (!refused && t !== null && t !== formatOffset(offset.steps)) await commitSectionOffset(t);
    cancelOffsetEdit();
  }
</script>

<div class="flex flex-wrap items-center gap-2" role="group" aria-label="Section start">
  <label for="section-offset" class="text-base">Start</label>
  <input
    id="section-offset"
    type="text"
    class="h-9 w-16 rounded-md border border-input bg-background px-1 text-center text-lg tabular-nums disabled:opacity-50"
    aria-invalid={invalid ? 'true' : undefined}
    disabled={!hasTempo}
    title={fieldTitle}
    data-testid="section-offset"
    value={text ?? formatOffset(offset.steps)}
    {onfocus}
    {oninput}
    {onkeydown}
    {onblur}
  />
  <span class="text-base text-muted-foreground">/64 beat</span>
  <span class="text-base tabular-nums" data-testid="section-offset-time">{offsetTime(offset.ms)}</span>
  <Button
    variant="outline"
    class="h-9 px-1.5 text-sm"
    aria-label="Find where the music starts and put the first beat there"
    disabled={noFind}
    title={findTitle}
    onclick={() => void findStart()}
  >Find start</Button>
</div>
