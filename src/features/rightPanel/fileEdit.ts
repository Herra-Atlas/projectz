/**
 * An in-progress rename or create in the file tree.
 *
 * One value rather than a handful of flags, because the tree shows at most one
 * inline name field at a time and each variant already says exactly where it
 * belongs: `rename` replaces one row, and `create` inserts a row as the first
 * child of `parent` (the empty string being the workspace root). Keeping it a
 * discriminated union means a row can decide whether it is the subject by
 * reading a field instead of comparing three booleans.
 */
export type FileEdit =
  | { kind: "rename"; path: string; name: string; isDir: boolean }
  | { kind: "create"; parent: string; isDir: boolean };
