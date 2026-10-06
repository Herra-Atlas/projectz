import { Blocks, FileText, FolderOpen, Globe, Pencil, Search, Terminal } from "lucide-react";

/**
 * The mark for a tool, so a row is recognisable before it is read.
 *
 * Its own file because two rows need it: the panel's own tool rows, and the
 * summary row that opens onto the files a reply changed. Sharing the component
 * is the point -- a summary row wearing a different mark to the rows it
 * summarises would read as a different kind of thing, which is exactly the
 * separation this row is meant not to have.
 */
export default function ToolIcon({ tool, size = 12 }: { tool: string; size?: number }) {
  const props = { size, className: "shrink-0 text-[var(--quiet)]" };
  if (tool === "read_file") return <FileText {...props} />;
  if (tool === "list_dir") return <FolderOpen {...props} />;
  if (tool === "search_files") return <Search {...props} />;
  if (tool === "search_web") return <Globe {...props} />;
  if (tool === "run_terminal") return <Terminal {...props} />;
  if (tool === "write_file" || tool === "edit_file") return <Pencil {...props} />;
  // A tool with no mark of its own still needs one, so an unknown step reads as
  // a step rather than as a stray line of text.
  return <Blocks {...props} />;
}