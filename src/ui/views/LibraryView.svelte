<script lang="ts">
  import ViewPlaceholder from '../components/ViewPlaceholder.svelte';
  import { VIEWS } from '$lib/views';
  const view = VIEWS.find((v) => v.id === 'library')!;
</script>

<ViewPlaceholder {view} />
