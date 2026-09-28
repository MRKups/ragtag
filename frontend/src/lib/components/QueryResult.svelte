<script lang="ts">
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';
  import type { Citation } from '$lib/types';
  import { api } from '$lib/api';
  import CitationList from './CitationList.svelte';

  let {
    text = '',
    citations = [] as Citation[],
    isStreaming = false,
    onCitationClick = (_citation: { filename: string; storagePath: string; documentId: string; page: number }) => {}
  } = $props<{
    text?: string;
    citations?: Citation[];
    isStreaming?: boolean;
    onCitationClick?: (citation: { filename: string; storagePath: string; documentId: string; page: number }) => void;
  }>();

  function transformCitations(html: string, citationsList: Citation[]): string {
    if (!citationsList.length || !html.includes('[')) return html;

    try {
      const parser = new DOMParser();
      const doc = parser.parseFromString(html, 'text/html');

      const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
      const textNodes: Text[] = [];
      while (walker.nextNode()) {
        const node = walker.currentNode as Text;
        const parentTag = node.parentElement?.tagName.toLowerCase();
        if (
          parentTag !== 'code' &&
          parentTag !== 'pre' &&
          parentTag !== 'script' &&
          parentTag !== 'style' &&
          parentTag !== 'button'
        ) {
          textNodes.push(node);
        }
      }

      const bracketRegex = /\[(?:Source\s*)?(\d+)(?:\s*,\s*(?:Source\s*)?(\d+))*\]/gi;

      for (const node of textNodes) {
        const textVal = node.nodeValue;
        if (!textVal || !textVal.includes('[')) continue;

        if (!bracketRegex.test(textVal)) continue;
        bracketRegex.lastIndex = 0;

        const fragment = doc.createDocumentFragment();
        let lastIndex = 0;
        let match: RegExpExecArray | null;

        while ((match = bracketRegex.exec(textVal)) !== null) {
          if (match.index > lastIndex) {
            fragment.appendChild(doc.createTextNode(textVal.slice(lastIndex, match.index)));
          }

          const inside = match[0].slice(1, -1);
          const numbers = inside.match(/\d+/g) || [];

          let anyPillAdded = false;
          for (const numStr of numbers) {
            const num = parseInt(numStr, 10);
            if (num >= 1 && num <= citationsList.length) {
              const cit = citationsList[num - 1];
              const btn = doc.createElement('button');
              btn.type = 'button';
              btn.className = 'citation-pill';
              btn.setAttribute('data-citation-id', String(num - 1));
              btn.setAttribute('title', `${cit.filename} (Page ${cit.page})`);
              btn.textContent = `[${num}]`;
              fragment.appendChild(btn);
              anyPillAdded = true;
            }
          }

          if (!anyPillAdded) {
            fragment.appendChild(doc.createTextNode(match[0]));
          }

          lastIndex = bracketRegex.lastIndex;
        }

        if (lastIndex < textVal.length) {
          fragment.appendChild(doc.createTextNode(textVal.slice(lastIndex)));
        }

        node.parentNode?.replaceChild(fragment, node);
      }

      return doc.body.innerHTML;
    } catch {
      return html;
    }
  }

  let renderedHtml = $derived.by(() => {
    if (!text) return '';
    const parsedHtml = marked.parse(text) as string;
    const sanitized = DOMPurify.sanitize(parsedHtml);
    return transformCitations(sanitized, citations);
  });

  function handleProseClick(event: MouseEvent) {
    const target = event.target as HTMLElement | null;
    const citationBtn = target?.closest('[data-citation-id]');
    if (citationBtn) {
      event.preventDefault();
      event.stopPropagation();
      const idx = parseInt(citationBtn.getAttribute('data-citation-id') || '', 10);
      const citation = citations[idx];
      if (citation) {
        onCitationClick({
          filename: citation.filename,
          storagePath: citation.storage_path,
          documentId: citation.document_id,
          page: citation.page
        });
      }
      return;
    }

    const anchor = target?.closest('a');
    if (anchor && anchor.href) {
      event.preventDefault();
      event.stopPropagation();
      try {
        const parsed = new URL(anchor.href);
        if (parsed.protocol === 'http:' || parsed.protocol === 'https:' || parsed.protocol === 'mailto:') {
          api.openExternalUrl(anchor.href).catch((err) => {
            console.error('Failed to open external link:', err);
          });
        }
      } catch {
        // Ignore malformed URLs; navigation already prevented
      }
    }
  }
</script>

<div class="result-container" class:has-content={!!text || isStreaming}>

  {#if text || isStreaming}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="prose" onclick={handleProseClick}>
      {@html renderedHtml}
      {#if isStreaming}
        <span class="cursor-blink">|</span>
      {/if}
    </div>
  {/if}

  {#if citations.length > 0 && !isStreaming}
    <CitationList {citations} {onCitationClick} />
  {/if}
</div>

<style>
  .result-container {
    width: 100%;
  }

  .result-container.has-content {
    background: #1f2937;
    border-radius: 1rem;
    box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.3), 0 2px 4px -1px rgba(0, 0, 0, 0.1);
    padding: 2rem;
    border: 1px solid #374151;
    box-sizing: border-box;
  }

  .prose {
    color: #d1d5db;
    line-height: 1.7;
    font-size: 1.05rem;
    overflow-wrap: break-word;
    word-break: break-word;
  }

  .prose :global(p) {
    margin-bottom: 1.25em;
    margin-top: 0;
  }

  .prose :global(ul), .prose :global(ol) {
    margin-bottom: 1.25em;
    padding-left: 1.5em;
    margin-top: 0;
  }

  .prose :global(li) {
    margin-bottom: 0.5em;
  }

  .prose :global(strong) {
    color: #f9fafb;
    font-weight: 600;
  }

  .prose :global(h1), .prose :global(h2), .prose :global(h3), .prose :global(h4) {
    color: #f3f4f6;
    font-weight: 600;
    margin-top: 1.5em;
    margin-bottom: 0.75em;
  }

  .prose :global(code) {
    background-color: #374151;
    padding: 0.2em 0.4em;
    border-radius: 0.25em;
    font-family: monospace;
    font-size: 0.9em;
  }

  .prose :global(pre) {
    background-color: #111827;
    padding: 1em;
    border-radius: 0.5em;
    overflow-x: auto;
    margin-bottom: 1.25em;
    border: 1px solid #374151;
  }

  .prose :global(pre code) {
    background-color: transparent;
    padding: 0;
    border-radius: 0;
    color: #e5e7eb;
  }

  .cursor-blink {
    animation: blink 1s step-end infinite;
    color: #3b82f6;
    font-weight: bold;
  }

  @keyframes blink {
    50% { opacity: 0; }
  }

  .prose :global(.citation-pill) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin: 0 0.15rem;
    padding: 0.1rem 0.4rem;
    font-size: 0.75rem;
    font-weight: 600;
    line-height: 1.2;
    color: #93c5fd;
    background: rgba(30, 58, 138, 0.45);
    border: 1px solid rgba(96, 165, 250, 0.4);
    border-radius: 0.25rem;
    cursor: pointer;
    vertical-align: baseline;
    transition: all 120ms ease-out;
    text-decoration: none;
    font-family: inherit;
  }

  .prose :global(.citation-pill:hover) {
    color: #ffffff;
    background: #2563eb;
    border-color: #60a5fa;
    transform: translateY(-1px);
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
  }

  .prose :global(.citation-pill:active) {
    transform: translateY(0);
  }
</style>
