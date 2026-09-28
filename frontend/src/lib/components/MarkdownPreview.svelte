<script lang="ts">
  import type { MarkdownBlock, MarkdownInline } from '$lib/types';
  import { api } from '$lib/api';

  let { blocks } = $props<{ blocks: MarkdownBlock[] }>();

  function headingTag(level: number) {
    return `h${Math.min(6, Math.max(1, level))}`;
  }

  function getInlineText(items: MarkdownInline[]): string {
    return items
      .map((item) => {
        if (item.kind === 'text' || item.kind === 'code') return item.content;
        if ('content' in item && Array.isArray(item.content)) return getInlineText(item.content);
        return '';
      })
      .join('');
  }

  function slugify(text: string): string {
    return text
      .toLowerCase()
      .trim()
      .replace(/[^\w\s-]/g, '')
      .replace(/[\s_-]+/g, '-')
      .replace(/^-+|-+$/g, '');
  }

  function isSafeHref(href: string): boolean {
    const trimmed = href.trim().toLowerCase();
    return (
      trimmed.startsWith('https://') ||
      trimmed.startsWith('http://') ||
      trimmed.startsWith('mailto:') ||
      trimmed.startsWith('#')
    );
  }

  function handleLinkClick(event: MouseEvent, href: string) {
    event.preventDefault();
    event.stopPropagation();
    const trimmed = href.trim();
    if (trimmed.startsWith('#')) {
      const targetId = trimmed.slice(1);
      if (targetId) {
        const element = document.getElementById(targetId);
        element?.scrollIntoView({ behavior: 'smooth' });
      }
      return;
    }
    const lower = trimmed.toLowerCase();
    if (lower.startsWith('https://') || lower.startsWith('http://') || lower.startsWith('mailto:')) {
      api.openExternalUrl(trimmed).catch((err) => {
        console.error('Failed to open external url:', err);
      });
    }
  }
</script>

{#snippet renderInlines(items: MarkdownInline[])}
  {#each items as item}
    {#if item.kind === 'text'}
      {item.content}
    {:else if item.kind === 'code'}
      <code>{item.content}</code>
    {:else if item.kind === 'emphasis'}
      <em>{@render renderInlines(item.content)}</em>
    {:else if item.kind === 'strong'}
      <strong>{@render renderInlines(item.content)}</strong>
    {:else if item.kind === 'strikethrough'}
      <s>{@render renderInlines(item.content)}</s>
    {:else if item.kind === 'link'}
      {#if isSafeHref(item.href)}
        <a href={item.href} onclick={(e) => handleLinkClick(e, item.href)}>{@render renderInlines(item.content)}</a>
      {:else}
        {@render renderInlines(item.content)}
      {/if}
    {:else if item.kind === 'break'}
      <br />
    {/if}
  {/each}
{/snippet}

{#snippet renderBlocks(items: MarkdownBlock[])}
  {#each items as block}
    {#if block.kind === 'heading'}
      {@const slug = slugify(getInlineText(block.content))}
      <svelte:element this={headingTag(block.level)} id={slug || undefined}>{@render renderInlines(block.content)}</svelte:element>
    {:else if block.kind === 'paragraph'}
      <p>{@render renderInlines(block.content)}</p>
    {:else if block.kind === 'block_quote'}
      <blockquote>{@render renderBlocks(block.blocks)}</blockquote>
    {:else if block.kind === 'list'}
      {#if block.start === null}
        <ul>
          {#each block.items as item}
            <li>{@render renderBlocks(item)}</li>
          {/each}
        </ul>
      {:else}
        <ol start={block.start}>
          {#each block.items as item}
            <li>{@render renderBlocks(item)}</li>
          {/each}
        </ol>
      {/if}
    {:else if block.kind === 'code_block'}
      <pre><code class:has-language={block.language !== null}>{block.content}</code></pre>
    {:else if block.kind === 'table'}
      <div class="table-scroll">
        <table>
          {#if block.header.length > 0}
            <thead>
              <tr>
                {#each block.header as cell}
                  <th>{@render renderInlines(cell)}</th>
                {/each}
              </tr>
            </thead>
          {/if}
          <tbody>
            {#each block.rows as row}
              <tr>
                {#each row as cell}
                  <td>{@render renderInlines(cell)}</td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {:else if block.kind === 'thematic_break'}
      <hr />
    {/if}
  {/each}
{/snippet}

<div class="markdown-preview">
  {@render renderBlocks(blocks)}
</div>

<style>
  .markdown-preview {
    color: #d1d5db;
    line-height: 1.6;
  }

  .markdown-preview :global(h1),
  .markdown-preview :global(h2),
  .markdown-preview :global(h3),
  .markdown-preview :global(h4),
  .markdown-preview :global(h5),
  .markdown-preview :global(h6) {
    color: #f3f4f6;
    line-height: 1.25;
    margin: 1.5rem 0 0.75rem;
  }

  .markdown-preview :global(h1) { font-size: 1.75rem; }
  .markdown-preview :global(h2) { font-size: 1.5rem; }
  .markdown-preview :global(h3) { font-size: 1.25rem; }
  .markdown-preview :global(h4),
  .markdown-preview :global(h5),
  .markdown-preview :global(h6) { font-size: 1.05rem; }

  .markdown-preview :global(p) {
    margin: 0 0 1rem;
  }

  .markdown-preview :global(a) {
    color: #60a5fa;
    text-decoration: underline;
  }

  .markdown-preview :global(code) {
    border-radius: 0.25rem;
    background: #111827;
    color: #e5e7eb;
    padding: 0.1rem 0.3rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
  }

  .markdown-preview :global(pre) {
    overflow-x: auto;
    border: 1px solid #334155;
    border-radius: 0.5rem;
    background: #111827;
    margin: 0 0 1rem;
    padding: 1rem;
  }

  .markdown-preview :global(pre code) {
    background: transparent;
    padding: 0;
  }

  .markdown-preview :global(blockquote) {
    border-left: 3px solid #64748b;
    color: #cbd5e1;
    margin: 0 0 1rem;
    padding-left: 1rem;
  }

  .markdown-preview :global(ul),
  .markdown-preview :global(ol) {
    margin: 0 0 1rem;
    padding-left: 1.5rem;
  }

  .markdown-preview :global(li > p:last-child) {
    margin-bottom: 0.25rem;
  }

  .markdown-preview :global(hr) {
    border: 0;
    border-top: 1px solid #334155;
    margin: 1.5rem 0;
  }

  .table-scroll {
    margin: 0 0 1rem;
    overflow-x: auto;
  }

  table {
    border-collapse: collapse;
    min-width: 100%;
  }

  th,
  td {
    border: 1px solid #334155;
    padding: 0.5rem 0.75rem;
    text-align: left;
    vertical-align: top;
  }

  th {
    background: #1e293b;
    color: #f3f4f6;
  }
</style>
