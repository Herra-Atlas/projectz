/**
 * The typography for rendered markdown, as a style tag's content.
 *
 * Extracted from the chat page because a second surface now renders markdown --
 * the Sub agents panel, which shows a sub-agent's answer the same way the chat
 * shows a reply. The rules belong to the *content*, not to the page it happens to
 * be on, so a copy per surface would be the same rule maintained twice and styled
 * two ways the first time one was touched.
 *
 * Injected as a `<style>` element rather than living in `App.css` because it is
 * scoped to a class that only rendered markdown carries, and because keeping it
 * beside the component that renders it is what makes "this is how a reply looks"
 * findable from that component.
 *
 * Colours come from the app's tokens (`--accent`, `--line`, `--rail`, ...) so the
 * block restyles with the theme and introduces no colour of its own.
 */
export const MARKDOWN_STYLES = `.markdown-content > :first-child{margin-top:0}.markdown-content > :last-child{margin-bottom:0}.markdown-content p{margin:.65rem 0;line-height:1.75}.markdown-content h1,.markdown-content h2,.markdown-content h3{margin:1.1rem 0 .5rem;font-weight:600}.markdown-content h1{font-size:1.35rem}.markdown-content h2{font-size:1.2rem}.markdown-content h3{font-size:1.05rem}.markdown-content ul,.markdown-content ol{margin:.65rem 0;padding-left:1.5rem}.markdown-content ul{list-style:disc}.markdown-content ol{list-style:decimal}.markdown-content li{padding-left:.2rem}.markdown-content a{color:var(--accent);text-decoration:underline;text-underline-offset:3px}.markdown-content blockquote{margin:.8rem 0;border-left:2px solid var(--accent);padding-left:1rem;color:var(--muted)}.markdown-content :not(pre)>code{border:1px solid var(--line);border-radius:4px;background:var(--raised);padding:.12rem .35rem;font-family:ui-monospace,SFMono-Regular,monospace;font-size:.9em}.markdown-content pre{max-width:100%;max-height:24rem;overflow:auto;overscroll-behavior:contain;border:1px solid var(--line);border-radius:8px;background:var(--rail);padding:1rem}.markdown-content pre code{font-family:ui-monospace,SFMono-Regular,monospace;font-size:.9em}.markdown-content table{display:block;max-width:100%;overflow-x:auto;border-collapse:collapse}.markdown-content th,.markdown-content td{border:1px solid var(--line);padding:.4rem .65rem;text-align:left}.markdown-content th{background:var(--raised)}.markdown-content hr{margin:1rem 0;border-color:var(--line)}`;
