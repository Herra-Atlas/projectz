import { AccessSwitches } from "../AccessSwitches";
import { Segmented, Toggle } from "../SettingsSection";
import type {
  MessagingPermission,
  MessagingSettings,
  MessagingUser,
} from "../../features/communications/types";

const PERMISSION_OPTIONS = ["Ask", "Auto reads", "Auto writes", "Full"] as const;

function permissionFor(label: (typeof PERMISSION_OPTIONS)[number]): MessagingPermission {
  if (label === "Ask") return "ask";
  if (label === "Auto reads") return "auto_safe";
  if (label === "Auto writes") return "auto_writes";
  return "full";
}

function permissionLabel(permission: MessagingPermission): (typeof PERMISSION_OPTIONS)[number] {
  if (permission === "ask") return "Ask";
  if (permission === "auto_safe") return "Auto reads";
  if (permission === "auto_writes") return "Auto writes";
  return "Full";
}

/**
 * One policy per allowed Telegram id.
 *
 * The numeric id is the identity the transport trusts, so this is where its
 * access belongs. The installation limit on the same page can only narrow these
 * switches, never be widened by them.
 */
export function TelegramUserAccess({
  settings,
  onChange,
}: {
  settings: MessagingSettings;
  onChange: (users: Record<string, MessagingUser>) => void;
}) {
  if (settings.allowedUserIds.length === 0) {
    return (
      <p className="text-[12px] leading-5 text-[var(--quiet)]">
        Add a Telegram user id above to give that person access.
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {settings.allowedUserIds.map((id) => {
        const user = settings.users[id];
        if (!user) return null;
        const update = (patch: Partial<MessagingUser>) =>
          onChange({ ...settings.users, [id]: { ...user, ...patch } });
        const enabled = Object.values(user.access).filter(Boolean).length;

        return (
          <details
            key={id}
            className="group rounded-lg border border-[var(--line)] bg-[var(--panel)]"
          >
            <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2.5">
              <span className="text-[12px] font-medium tabular-nums text-[var(--text)]">
                {id}
              </span>
              <span className="text-[11px] text-[var(--quiet)]">
                {user.allowAgent ? "Agent" : "Chat"} · {enabled} tools
              </span>
            </summary>

            <div className="flex flex-col gap-3 border-t border-[var(--line)] px-3 py-3">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="text-[12px] font-medium text-[var(--text)]">Allow Agent</p>
                  <p className="mt-0.5 text-[11px] leading-4 text-[var(--quiet)]">
                    Off forces Chat for this person, even in an older Agent session.
                  </p>
                </div>
                <Toggle
                  checked={user.allowAgent}
                  label={`Allow Agent for ${id}`}
                  onChange={(allowAgent) =>
                    update({
                      allowAgent,
                      ...(allowAgent ? {} : { mode: "chat" as const }),
                    })
                  }
                />
              </div>

              {user.allowAgent ? (
                <div className="flex items-center justify-between gap-3">
                  <div>
                    <p className="text-[12px] font-medium text-[var(--text)]">Default mode</p>
                    <p className="mt-0.5 text-[11px] leading-4 text-[var(--quiet)]">
                      Used when this person starts a conversation without choosing.
                    </p>
                  </div>
                  <Segmented
                    label={`Default mode for ${id}`}
                    value={user.mode === "agent" ? "Agent" : "Chat"}
                    options={["Agent", "Chat"] as const}
                    onChange={(label) =>
                      update({ mode: label === "Agent" ? "agent" : "chat" })
                    }
                  />
                </div>
              ) : null}

              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="text-[12px] font-medium text-[var(--text)]">Permission</p>
                  <p className="mt-0.5 text-[11px] leading-4 text-[var(--quiet)]">
                    What may run without a prompt. Approvals still arrive as buttons.
                  </p>
                </div>
                <Segmented
                  label={`Permission for ${id}`}
                  value={permissionLabel(user.permission)}
                  options={PERMISSION_OPTIONS}
                  onChange={(label) => update({ permission: permissionFor(label) })}
                />
              </div>

              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="text-[12px] font-medium text-[var(--text)]">
                    Allow to start local model
                  </p>
                  <p className="mt-0.5 text-[11px] leading-4 text-[var(--quiet)]">
                    When their local chat model is not loaded, the chat may offer Start.
                  </p>
                </div>
                <Toggle
                  checked={user.allowLocalStart}
                  label={`Allow local model start for ${id}`}
                  onChange={(allowLocalStart) => update({ allowLocalStart })}
                />
              </div>

              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="text-[12px] font-medium text-[var(--text)]">
                    Allow to start speech-to-text
                  </p>
                  <p className="mt-0.5 text-[11px] leading-4 text-[var(--quiet)]">
                    Lets their voice messages start the selected STT engine and be transcribed.
                  </p>
                </div>
                <Toggle
                  checked={user.allowSttStart}
                  label={`Allow speech-to-text start for ${id}`}
                  onChange={(allowSttStart) => update({ allowSttStart })}
                />
              </div>

              <div className="flex flex-col gap-2">
                <div>
                  <p className="text-[12px] font-medium text-[var(--text)]">Tools</p>
                  <p className="mt-0.5 text-[11px] leading-4 text-[var(--quiet)]">
                    An off tool is hidden from this person's runs. The installation limit
                    can still remove more.
                  </p>
                </div>
                <AccessSwitches
                  access={user.access}
                  onChange={(access) => update({ access })}
                  labelPrefix={`${id} `}
                />
              </div>
            </div>
          </details>
        );
      })}
    </div>
  );
}
