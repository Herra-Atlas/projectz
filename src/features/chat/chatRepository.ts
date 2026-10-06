import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ChatSession, ChatSessionHeader } from "./types";

export interface ChatRepository {
  initialize(): Promise<void>;
  list(): Promise<ChatSession[]>;
  /**
   * Every conversation, without any messages.
   *
   * The launch read. A session list draws titles, ordering and a dot, none of
   * which need a transcript, so fetching every message in the database to render
   * a list of titles made startup cost grow with history rather than with the
   * number of conversations.
   */
  listHeaders(): Promise<ChatSessionHeader[]>;
  /**
   * One conversation's messages, oldest first.
   *
   * Called when a conversation is opened. An empty array means the conversation
   * genuinely has no messages -- a session that was created and never sent to is
   * a real state, and it is different from not having asked yet.
   */
  listMessages(id: string): Promise<ChatMessage[]>;
  save(session: ChatSession): Promise<void>;
  /**
   * Saves everything except the transcript.
   *
   * A rename, a pin, a dot colour and a generated title change no messages, so
   * they must not send any. `save_chat_session_tx` treats an absent `messages`
   * key as "not writing messages" and leaves every row alone, whereas sending
   * the array would mean rewriting the whole transcript to change a title.
   *
   * This is the reason the split is safe: a header-only write cannot truncate a
   * conversation, because it does not carry a length to truncate to.
   */
  saveHeader(header: ChatSessionHeader): Promise<void>;
  remove(id: string): Promise<void>;
  /** Deletes every conversation and resolves with how many were removed. */
  clear(): Promise<number>;
}

const STORAGE_KEY = "projectz.chat-sessions.v1";
class SqliteChatRepository implements ChatRepository {
  async initialize(): Promise<void> {
    try {
      if (await invoke<boolean>("database_frontend_imported")) return;
      const legacy = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]") as ChatSession[];
      const general = JSON.parse(localStorage.getItem("projectz.general-settings") ?? "{}");
      await invoke("database_import_frontend", { sessions: legacy, generalSettings: general });
    } catch (error) {
      throw new Error(`Could not migrate existing ProjectZ data: ${String(error)}`);
    }
  }

  list(): Promise<ChatSession[]> {
    return invoke<ChatSession[]>("database_list_sessions");
  }

  listHeaders(): Promise<ChatSessionHeader[]> {
    return invoke<ChatSessionHeader[]>("database_list_session_headers");
  }

  listMessages(id: string): Promise<ChatMessage[]> {
    // `sessionId` in camelCase because that is what Tauri matches the Rust
    // `session_id` argument against on the way in.
    return invoke<ChatMessage[]>("database_list_session_messages", { sessionId: id });
  }

  save(session: ChatSession): Promise<void> {
    return invoke("database_save_session", { session });
  }

  saveHeader(header: ChatSessionHeader): Promise<void> {
    // The `messages` key is absent by construction. `ChatSessionHeader` has no
    // such property, so there is no way for a caller to smuggle a length along.
    return invoke("database_save_session", { session: header });
  }

  remove(id: string): Promise<void> {
    return invoke("database_delete_session", { id });
  }

  clear(): Promise<number> {
    return invoke<number>("database_clear_sessions");
  }
}

export const chatRepository: ChatRepository = new SqliteChatRepository();
