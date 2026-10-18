import {
  Blocks,
  CircleHelp,
  FileOutput,
  FileSearch,
  FileText,
  FolderOpen,
  Globe,
  ListChecks,
  Move,
  Pencil,
  Search,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";

/**
 * The mark for a tool, so a row is recognisable before it is read.
 *
 * Its own file because two rows need it: the panel's own tool rows, and the
 * summary row that opens onto the files a reply changed. Sharing the component
 * is the point -- a summary row wearing a different mark to the rows it
 * summarises would read as a different kind of thing, which is exactly the
 * separation this row is meant not to have.
 *
 * The names are the backend's tool names, so every tool in `registry.rs` has a
 * mark here. A tool with none still gets a mark rather than a blank, but a named
 * one is what keeps two rows apart at a glance.
 */
export default function ToolIcon({
  tool,
  size = 12,
  className = "text-[var(--quiet)]",
}: {
  tool: string;
  size?: number;
  /**
   * The mark's colour, defaulting to the row's quiet tone.
   *
   * Overridable so a failed call can tint its icon red without a second icon
   * component: the failure should be visible on the mark itself, not only in the
   * text beside it.
   */
  className?: string;
}) {
  const props = { size, className: `shrink-0 ${className}` };
  if (tool === "read_file") return <FileText {...props} />;
  if (tool === "list_dir") return <FolderOpen {...props} />;
  if (tool === "grep") return <Search {...props} />;
  if (tool === "glob") return <FileSearch {...props} />;
  if (tool === "search_web" || tool === "web_fetch") return <Globe {...props} />;
  if (tool === "run_terminal" || tool === "terminal_output" || tool === "terminal_status")
    return <Terminal {...props} />;
  if (tool === "terminal_kill") return <Square {...props} />;
  if (tool === "write_file" || tool === "edit_file" || tool === "edit_lines") return <Pencil {...props} />;
  if (tool === "write_document") return <FileOutput {...props} />;
  if (tool === "move_file") return <Move {...props} />;
  if (tool === "delete_file") return <Trash2 {...props} />;
  if (tool === "ask_user") return <CircleHelp {...props} />;
  if (tool === "todo") return <ListChecks {...props} />;
  // A tool with no mark of its own still needs one, so an unknown step reads as
  // a step rather than as a stray line of text.
  return <Blocks {...props} />;
}