/**
 * The typography a rendered document is given.
 *
 * Rendered Markdown and converted Word HTML are the same kind of thing on screen:
 * headings, paragraphs, lists, tables and code, with none of the app's own chrome.
 * One style block rather than two, so a change to how a document reads applies to
 * both -- and the class name stays `markdown-content` because the chat's replies
 * already use it, which is what makes a previewed file look like an answer.
 *
 * Inlined rather than put in the stylesheet because it is scoped to these two
 * renderers; a global rule would apply the same typography to the app's own UI.
 */
export const CONTENT_STYLES = `.markdown-content > :first-child{margin-top:0}.markdown-content > :last-child{margin-bottom:0}.markdown-content p{margin:.65rem 0;line-height:1.75}.markdown-content h1,.markdown-content h2,.markdown-content h3{margin:1.1rem 0 .5rem;font-weight:600}.markdown-content h1{font-size:1.35rem}.markdown-content h2{font-size:1.2rem}.markdown-content h3{font-size:1.05rem}.markdown-content ul,.markdown-content ol{margin:.65rem 0;padding-left:1.5rem}.markdown-content ul{list-style:disc}.markdown-content ol{list-style:decimal}.markdown-content li{padding-left:.2rem}.markdown-content a{color:var(--accent);text-decoration:underline;text-underline-offset:3px}.markdown-content blockquote{margin:.8rem 0;border-left:2px solid var(--accent);padding-left:1rem;color:var(--muted)}.markdown-content :not(pre)>code{border:1px solid var(--line);border-radius:4px;background:var(--raised);padding:.12rem .35rem;font-family:ui-monospace,SFMono-Regular,monospace;font-size:.9em}.markdown-content pre{max-width:100%;max-height:24rem;overflow:auto;overscroll-behavior:contain;border:1px solid var(--line);border-radius:8px;background:var(--rail);padding:1rem}.markdown-content pre code{font-family:ui-monospace,SFMono-Regular,monospace;font-size:.9em}.markdown-content table{display:block;max-width:100%;overflow-x:auto;border-collapse:collapse}.markdown-content th,.markdown-content td{border:1px solid var(--line);padding:.4rem .65rem;text-align:left}.markdown-content th{background:var(--raised)}.markdown-content hr{margin:1rem 0;border-color:var(--line)}.markdown-content img{max-width:100%;height:auto}`;

/** The class the styles above are written for. */
export const CONTENT_CLASS = "markdown-content";
