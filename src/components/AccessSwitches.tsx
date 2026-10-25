import { Toggle } from "./SettingsSection";
import {
  ACCESS_CAPABILITIES,
  ACCESS_LABELS,
  type AccessCapability,
  type AccessSet,
} from "../features/chat/access";

/**
 * The capability switches, in the one layout jobs, Telegram and the host all
 * use.
 *
 * `capabilities` is how a caller removes a switch that does not apply to its
 * run -- a job's own run can never schedule more jobs, so offering "Jobs" there
 * would be a control that changes nothing.
 */
export function AccessSwitches({
  access,
  onChange,
  capabilities = ACCESS_CAPABILITIES,
  labelPrefix = "",
}: {
  access: AccessSet;
  onChange: (access: AccessSet) => void;
  capabilities?: AccessCapability[];
  /** Distinguishes the same labels when two editors are on one page. */
  labelPrefix?: string;
}) {
  return (
    <div className="flex flex-wrap gap-x-5 gap-y-3 text-[13px]">
      {capabilities.map((capability) => {
        const label = ACCESS_LABELS[capability];
        return (
          <label key={capability} className="flex items-center gap-2 text-[var(--text)]">
            <Toggle
              checked={access[capability]}
              onChange={(value) => onChange({ ...access, [capability]: value })}
              label={`${labelPrefix}${label}`}
            />
            {label}
          </label>
        );
      })}
    </div>
  );
}
