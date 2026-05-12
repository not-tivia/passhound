import type { Mapping } from "../types";

export type ColumnRole =
  | "site"
  | "username"
  | "display_name"
  | "password"
  | "url"
  | "notes"
  | "created_at"
  | "merge"
  | "skip";

const ROLE_OPTIONS: { value: ColumnRole; label: string }[] = [
  { value: "site", label: "site" },
  { value: "username", label: "username" },
  { value: "display_name", label: "display name" },
  { value: "password", label: "password" },
  { value: "url", label: "url" },
  { value: "notes", label: "notes" },
  { value: "created_at", label: "created_at" },
  { value: "merge", label: "merge into notes" },
  { value: "skip", label: "skip" },
];

/**
 * Single-index roles — at most one column can be mapped to each.
 * `merge` and `skip` are list-style and accept any number of columns.
 */
const SINGLE_INDEX_ROLES: ColumnRole[] = [
  "site",
  "username",
  "display_name",
  "password",
  "url",
  "notes",
  "created_at",
];

interface ColumnMappingTableProps {
  headers: string[];
  roles: ColumnRole[];                // length === headers.length
  onChange: (index: number, role: ColumnRole) => void;
}

export default function ColumnMappingTable({
  headers,
  roles,
  onChange,
}: ColumnMappingTableProps) {
  return (
    <div className="col-mapping">
      <div className="col-mapping__label">Detected columns</div>
      <table className="col-mapping__table">
        <thead>
          <tr>
            <th>CSV column</th>
            <th>Field</th>
          </tr>
        </thead>
        <tbody>
          {headers.map((h, i) => (
            <tr key={i}>
              <td>{h}</td>
              <td>
                <select
                  value={roles[i]}
                  onChange={(e) => onChange(i, e.target.value as ColumnRole)}
                  className="col-mapping__select"
                >
                  {ROLE_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * Validate that single-index roles have at most one column each and
 * that `password` has exactly one. Returns null if valid, else an error
 * message suitable for an Import button tooltip.
 */
export function validateRoles(roles: ColumnRole[]): string | null {
  const counts: Partial<Record<ColumnRole, number>> = {};
  for (const r of roles) counts[r] = (counts[r] ?? 0) + 1;
  for (const r of SINGLE_INDEX_ROLES) {
    if ((counts[r] ?? 0) > 1) {
      return `Exactly one column must be mapped to ${r}`;
    }
  }
  if ((counts.password ?? 0) !== 1) {
    return "Exactly one column must be mapped to password";
  }
  return null;
}

/**
 * Build a `Mapping` from the roles array. Caller must call `validateRoles`
 * first; this function assumes the roles are valid and panics otherwise.
 */
export function rolesToMapping(roles: ColumnRole[]): Mapping {
  const findOne = (target: ColumnRole): number | null => {
    const idx = roles.findIndex((r) => r === target);
    return idx >= 0 ? idx : null;
  };
  const password = findOne("password");
  if (password === null) {
    throw new Error("rolesToMapping called without a password column");
  }
  return {
    site: findOne("site"),
    url: findOne("url"),
    username: findOne("username"),
    display_name: findOne("display_name"),
    password,
    notes: findOne("notes"),
    created_at: findOne("created_at"),
    extras_into_notes: roles
      .map((r, i) => ({ role: r, index: i }))
      .filter((x) => x.role === "merge")
      .map((x) => x.index),
  };
}

/**
 * Derive initial roles from a backend-detected Mapping.
 * Columns referenced by the mapping take their role; columns referenced
 * by `extras_into_notes` become `merge`; everything else becomes `skip`.
 */
export function mappingToRoles(headers: string[], mapping: Mapping): ColumnRole[] {
  const roles: ColumnRole[] = headers.map(() => "skip");
  const assign = (index: number | null, role: ColumnRole) => {
    if (index !== null && index >= 0 && index < roles.length) {
      roles[index] = role;
    }
  };
  assign(mapping.site, "site");
  assign(mapping.url, "url");
  assign(mapping.username, "username");
  assign(mapping.display_name, "display_name");
  assign(mapping.password, "password");
  assign(mapping.notes, "notes");
  assign(mapping.created_at, "created_at");
  for (const idx of mapping.extras_into_notes) {
    if (idx >= 0 && idx < roles.length) roles[idx] = "merge";
  }
  return roles;
}
