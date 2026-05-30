<script lang="ts">
  import { CheckCircle, AlertTriangle, AlertOctagon } from 'lucide-svelte';

  let { level = 'LOW' } = $props<{
    level?: 'HIGH' | 'MEDIUM' | 'LOW';
  }>();

  const config: Record<string, { className: string; icon: typeof CheckCircle; label: string }> = {
    HIGH: {
      className: 'badge-high',
      icon: CheckCircle,
      label: 'High Confidence'
    },
    MEDIUM: {
      className: 'badge-medium',
      icon: AlertTriangle,
      label: 'Medium Confidence'
    },
    LOW: {
      className: 'badge-low',
      icon: AlertOctagon,
      label: 'Low Confidence'
    }
  };

  let current = $derived(config[level] || config.LOW);
</script>

<div class="badge {current.className}">
  <svelte:component this={current.icon} size={16} />
  <span>{current.label}</span>
</div>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.25rem 0.75rem;
    border-radius: 9999px;
    border: 1px solid transparent;
    font-size: 0.875rem;
    font-weight: 500;
  }

  .badge-high {
    background-color: rgba(20, 83, 45, 0.2);
    color: #4ade80;
    border-color: #166534;
  }

  .badge-medium {
    background-color: rgba(113, 63, 18, 0.2);
    color: #facc15;
    border-color: #854d0e;
  }

  .badge-low {
    background-color: rgba(127, 29, 29, 0.2);
    color: #f87171;
    border-color: #991b1b;
  }
</style>
