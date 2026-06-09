<script lang="ts">
  let { score }: { score: number } = $props<{ score: number }>();

  let label = $derived(score >= 0.8 ? 'High' : score >= 0.5 ? 'Medium' : 'Low');
  let cls = $derived(score >= 0.8 ? 'high' : score >= 0.5 ? 'medium' : 'low');
</script>

<span class="confidence-badge {cls}">{label}</span>

<style>
  .confidence-badge {
    padding: 0.1rem 0.4rem;
    border-radius: 0.25rem;
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .high {
    background: rgba(34, 197, 94, 0.15);
    color: #4ade80;
  }

  .medium {
    background: rgba(234, 179, 8, 0.15);
    color: #facc15;
  }

  .low {
    background: rgba(239, 68, 68, 0.15);
    color: #f87171;
  }
</style>
