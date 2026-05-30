  <script lang="ts">
  import { CheckCircle, AlertTriangle, AlertCircle } from 'lucide-svelte';

  let { confidence } = $props<{
    confidence: 'HIGH' | 'MEDIUM' | 'LOW';
  }>();

  let config: Record<'HIGH' | 'MEDIUM' | 'LOW', { icon: any, label: string, className: string }> = $derived({
    HIGH: {
      icon: CheckCircle,
      label: 'High Confidence',
      className: 'badge-high'
    },
    MEDIUM: {
      icon: AlertTriangle,
      label: 'Medium Confidence',
      className: 'badge-medium'
    },
    LOW: {
      icon: AlertCircle,
      label: 'Low Confidence',
      className: 'badge-low'
    }
  });

  let current = $derived(config[confidence as 'HIGH' | 'MEDIUM' | 'LOW']);
  let CurrentIcon = $derived(current.icon);
</script>

<div class="badge {current.className}">
  <CurrentIcon size={16} />
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
