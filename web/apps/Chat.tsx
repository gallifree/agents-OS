import { Suspense, useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  ConnectionHandler,
  fetchQuery,
  graphql,
  useLazyLoadQuery,
  useMutation,
  useRelayEnvironment,
  useSubscription,
} from "react-relay";
import { Bot, Hash, Lock, Plus, Send, Settings2 } from "lucide-react";
import type { AppProps } from "../desktop/apps";
import { errorCode } from "../desktop/errors";
import { StatusLine, type Status } from "../desktop/StatusLine";
import type { ChatChannelsQuery } from "./__generated__/ChatChannelsQuery.graphql";
import type { ChatChannelQuery } from "./__generated__/ChatChannelQuery.graphql";
import type { ChatOlderQuery, ChatOlderQuery$data } from "./__generated__/ChatOlderQuery.graphql";
import type { ChatPostMutation } from "./__generated__/ChatPostMutation.graphql";
import type { ChatMessagePostedSubscription } from "./__generated__/ChatMessagePostedSubscription.graphql";
import type { ChatAgentTypingSubscription } from "./__generated__/ChatAgentTypingSubscription.graphql";
import type { ChatCreateChannelMutation } from "./__generated__/ChatCreateChannelMutation.graphql";
import type { ChatUpdateChannelMutation } from "./__generated__/ChatUpdateChannelMutation.graphql";
import type { ChatDeleteChannelMutation } from "./__generated__/ChatDeleteChannelMutation.graphql";

const channelsQuery = graphql`
  query ChatChannelsQuery {
    channels {
      id
      cacheKey
      name
      private
    }
    agents {
      id
      cacheKey
      name
    }
  }
`;

const channelQuery = graphql`
  query ChatChannelQuery($id: ID!) {
    node(id: $id) {
      ... on Channel {
        id
        cacheKey
        name
        private
        canEdit
        agents {
          id
          cacheKey
          name
        }
        messages(last: 50) @connection(key: "ChatChannel_messages") {
          edges {
            node {
              id
              cacheKey
              authorName
              authorKind
              fromViewer
              content
              isError
              createdAt
            }
          }
          pageInfo {
            hasPreviousPage
            startCursor
          }
        }
      }
    }
  }
`;

const olderQuery = graphql`
  query ChatOlderQuery($id: ID!, $before: String!) {
    node(id: $id) {
      ... on Channel {
        cacheKey
        older: messages(last: 50, before: $before) {
          edges {
            node {
              id
              cacheKey
              authorName
              authorKind
              fromViewer
              content
              isError
              createdAt
            }
          }
          pageInfo {
            hasPreviousPage
            startCursor
          }
        }
      }
    }
  }
`;

const postMutation = graphql`
  mutation ChatPostMutation($channelId: ID!, $content: String!) {
    postMessage(channelId: $channelId, content: $content) {
      cacheKey
    }
  }
`;

const messagePostedSubscription = graphql`
  subscription ChatMessagePostedSubscription($channelId: ID!, $connections: [ID!]!) {
    messagePosted(channelId: $channelId) @appendNode(connections: $connections, edgeTypeName: "MessageEdge") {
      id
      cacheKey
      authorName
      authorKind
      fromViewer
      content
      isError
      createdAt
    }
  }
`;

const agentTypingSubscription = graphql`
  subscription ChatAgentTypingSubscription($channelId: ID!) {
    agentTyping(channelId: $channelId) {
      agent
      typing
    }
  }
`;

const createChannelMutation = graphql`
  mutation ChatCreateChannelMutation($name: String!, $private: Boolean!, $agentIds: [ID!]!) {
    createChannel(name: $name, private: $private, agentIds: $agentIds) {
      id
      cacheKey
    }
  }
`;

const updateChannelMutation = graphql`
  mutation ChatUpdateChannelMutation($id: ID!, $name: String!, $agentIds: [ID!]!) {
    updateChannel(id: $id, name: $name, agentIds: $agentIds) {
      cacheKey
      name
      agents {
        id
        cacheKey
        name
      }
    }
  }
`;

const deleteChannelMutation = graphql`
  mutation ChatDeleteChannelMutation($id: ID!) {
    deleteChannel(id: $id)
  }
`;

type AgentRef = { readonly id: string; readonly cacheKey: string; readonly name: string };
const errorText = (t: (k: string) => string, err: unknown) => t(`errors.${errorCode(err) ?? "NETWORK"}`);

export default function Chat(_: AppProps) {
  const { t } = useTranslation();
  const [fetchKey, setFetchKey] = useState(0);
  const { channels, agents } = useLazyLoadQuery<ChatChannelsQuery>(
    channelsQuery,
    {},
    { fetchKey, fetchPolicy: "store-and-network" },
  );
  // Selection by cacheKey: stable, while ids are re-issued on every fetch.
  const [selectedKey, setSelectedKey] = useState<string>();
  const [creating, setCreating] = useState(false);
  const selected = channels.find((c) => c.cacheKey === selectedKey) ?? (creating ? undefined : channels[0]);
  const refresh = () => setFetchKey((k) => k + 1);

  return (
    <div className="flex h-full max-sm:flex-col">
      <nav className="flex shrink-0 flex-col gap-1 border-base-300 p-2 sm:w-44 sm:border-r max-sm:max-h-32 max-sm:border-b">
        <button className="btn btn-ghost btn-sm justify-start" onClick={() => setCreating(true)}>
          <Plus size={14} /> {t("apps.chat.newChannel")}
        </button>
        <ul className="menu menu-sm w-full overflow-auto p-0">
          {channels.map((c) => (
            <li key={c.cacheKey}>
              <button
                className={!creating && selected?.cacheKey === c.cacheKey ? "menu-active" : ""}
                onClick={() => {
                  setCreating(false);
                  setSelectedKey(c.cacheKey);
                }}
              >
                {c.private ? <Lock size={12} /> : <Hash size={12} />}
                <span className="truncate">{c.name}</span>
              </button>
            </li>
          ))}
        </ul>
      </nav>
      <section className="flex min-h-0 min-w-0 flex-1 flex-col">
        {creating || !selected ? (
          <ChannelForm
            agents={agents}
            onDone={(cacheKey) => {
              setCreating(false);
              if (cacheKey) setSelectedKey(cacheKey);
              refresh();
            }}
            onCancel={channels.length > 0 ? () => setCreating(false) : undefined}
          />
        ) : (
          <Suspense fallback={<span className="loading loading-spinner m-4" />}>
            <ChannelView key={selected.cacheKey} id={selected.id} allAgents={agents} onChanged={refresh} />
          </Suspense>
        )}
      </section>
    </div>
  );
}

/** New channel, or the settings of an existing one when `channel` is given. */
function ChannelForm({
  agents,
  channel,
  onDone,
  onCancel,
}: {
  agents: readonly AgentRef[];
  channel?: { id: string; name: string; agents: readonly AgentRef[] };
  onDone: (cacheKey?: string) => void;
  onCancel?: () => void;
}) {
  const { t } = useTranslation();
  const [create, creating] = useMutation<ChatCreateChannelMutation>(createChannelMutation);
  const [update, updating] = useMutation<ChatUpdateChannelMutation>(updateChannelMutation);
  const [remove, removing] = useMutation<ChatDeleteChannelMutation>(deleteChannelMutation);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [status, setStatus] = useState<Status>();
  const onError = (err: unknown) => setStatus({ ok: false, text: errorText(t, err) });
  const members = new Set(channel?.agents.map((a) => a.cacheKey));

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const name = String(form.get("name"));
    const agentIds = agents.filter((a) => form.get(`agent-${a.cacheKey}`)).map((a) => a.id);
    if (channel) {
      update({ variables: { id: channel.id, name, agentIds }, onCompleted: () => onDone(), onError });
    } else {
      create({
        variables: { name, private: form.get("private") === "on", agentIds },
        onCompleted: (r) => onDone(r.createChannel.cacheKey),
        onError,
      });
    }
  };

  return (
    <form onSubmit={submit} className="max-w-sm overflow-auto p-4">
      <fieldset className="fieldset" disabled={creating || updating}>
        <legend className="fieldset-legend">{t(channel ? "apps.chat.channelSettings" : "apps.chat.newChannel")}</legend>
        <label className="fieldset-label" htmlFor="channel-name">
          {t("apps.models.name")}
        </label>
        <input id="channel-name" name="name" className="input w-full" required maxLength={64} defaultValue={channel?.name} />
        {!channel && (
          <label className="fieldset-label mt-2">
            <input name="private" type="checkbox" className="checkbox checkbox-sm" />
            {t("apps.chat.private")}
          </label>
        )}
        <p className="fieldset-label mt-2">{t("apps.chat.agentsInChannel")}</p>
        {agents.length === 0 && <p className="text-xs opacity-60">{t("apps.chat.noAgents")}</p>}
        {agents.map((a) => (
          <label key={a.cacheKey} className="fieldset-label">
            <input
              name={`agent-${a.cacheKey}`}
              type="checkbox"
              className="checkbox checkbox-sm"
              defaultChecked={members.has(a.cacheKey)}
            />
            @{a.name}
          </label>
        ))}
        <p className="text-xs opacity-60">{t("apps.chat.agentsHint")}</p>
        <div className="mt-2 flex gap-2">
          <button className="btn btn-primary btn-sm">{t(channel ? "apps.settings.save" : "apps.settings.create")}</button>
          {onCancel && (
            <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel}>
              {t("apps.chat.cancel")}
            </button>
          )}
          {channel && (
            <button
              type="button"
              className={`btn btn-sm ms-auto ${confirmDelete ? "btn-error" : "btn-ghost"}`}
              disabled={removing}
              onBlur={() => setConfirmDelete(false)}
              onClick={() =>
                confirmDelete
                  ? remove({ variables: { id: channel.id }, onCompleted: () => onDone(), onError })
                  : setConfirmDelete(true)
              }
            >
              {t(confirmDelete ? "apps.models.confirmDelete" : "apps.models.delete")}
            </button>
          )}
        </div>
      </fieldset>
      <StatusLine status={status} />
    </form>
  );
}

type MessageData = NonNullable<NonNullable<ChatOlderQuery$data["node"]>["older"]>["edges"][number]["node"];

/** Agents stop showing as typing after this long, in case their "done" event was lost. */
const TYPING_TIMEOUT_MS = 3 * 60_000;

function ChannelView({ id, allAgents, onChanged }: { id: string; allAgents: readonly AgentRef[]; onChanged: () => void }) {
  const { t, i18n } = useTranslation();
  const environment = useRelayEnvironment();
  const { node: channel } = useLazyLoadQuery<ChatChannelQuery>(channelQuery, { id }, { fetchPolicy: "store-and-network" });
  const [post, posting] = useMutation<ChatPostMutation>(postMutation);
  const [older, setOlder] = useState<MessageData[]>([]);
  const [olderCursor, setOlderCursor] = useState<{ has: boolean; cursor?: string | null }>();
  const [typing, setTyping] = useState<Record<string, number>>({});
  const [editing, setEditing] = useState(false);
  const [status, setStatus] = useState<Status>();
  const bottom = useRef<HTMLDivElement>(null);
  const time = new Intl.DateTimeFormat(i18n.language, { hour: "2-digit", minute: "2-digit" });

  const cacheKey = channel?.cacheKey;
  const connections = useMemo(
    () => (cacheKey ? [ConnectionHandler.getConnectionID(cacheKey, "ChatChannel_messages")] : []),
    [cacheKey],
  );
  useSubscription<ChatMessagePostedSubscription>(
    useMemo(
      () => ({
        subscription: messagePostedSubscription,
        variables: { channelId: id, connections },
        onNext: (r) => {
          const m = r?.messagePosted;
          if (m?.authorKind === "AGENT")
            setTyping(({ [m.authorName]: _, ...rest }) => rest);
        },
      }),
      [id, connections],
    ),
  );
  useSubscription<ChatAgentTypingSubscription>(
    useMemo(
      () => ({
        subscription: agentTypingSubscription,
        variables: { channelId: id },
        onNext: (r) => {
          const e = r?.agentTyping;
          if (!e) return;
          setTyping(({ [e.agent]: _, ...rest }) => (e.typing ? { ...rest, [e.agent]: Date.now() } : rest));
        },
      }),
      [id],
    ),
  );

  const messages = channel?.messages?.edges.map((e) => e.node) ?? [];
  const lastKey = messages.at(-1)?.cacheKey;
  const typingNames = Object.entries(typing)
    .filter(([, since]) => Date.now() - since < TYPING_TIMEOUT_MS)
    .map(([name]) => name);
  useEffect(() => bottom.current?.scrollIntoView({ block: "end" }), [lastKey, typingNames.length]);

  if (!channel?.messages) return <p className="p-4 opacity-70">{t("errors.NOT_FOUND")}</p>;
  const page = olderCursor ?? { has: channel.messages.pageInfo.hasPreviousPage, cursor: channel.messages.pageInfo.startCursor };

  const loadOlder = () => {
    if (!page.cursor) return;
    fetchQuery<ChatOlderQuery>(environment, olderQuery, { id, before: page.cursor })
      .toPromise()
      .then((r) => {
        const conn = r?.node?.older;
        if (!conn) return;
        setOlder((prev) => [...conn.edges.map((e) => e.node), ...prev]);
        setOlderCursor({ has: conn.pageInfo.hasPreviousPage, cursor: conn.pageInfo.startCursor });
      })
      .catch((err) => setStatus({ ok: false, text: errorText(t, err) }));
  };

  const send = (form: HTMLFormElement) => {
    const input = form.elements.namedItem("content") as HTMLTextAreaElement;
    const content = input.value.trim();
    if (!content) return;
    post({
      variables: { channelId: id, content },
      onCompleted: () => {
        input.value = "";
        setStatus(undefined);
      },
      onError: (err) => setStatus({ ok: false, text: errorText(t, err) }),
    });
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter sends, Shift+Enter adds a line; never while an IME composition is in progress.
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (e.currentTarget.form) send(e.currentTarget.form);
    }
  };

  if (editing)
    return (
      <ChannelForm
        agents={allAgents}
        channel={{ id: channel.id ?? id, name: channel.name ?? "", agents: channel.agents ?? [] }}
        onDone={() => {
          setEditing(false);
          onChanged();
        }}
        onCancel={() => setEditing(false)}
      />
    );

  return (
    <>
      <header className="flex items-center gap-2 border-b border-base-300 px-3 py-2">
        {channel.private ? <Lock size={14} /> : <Hash size={14} />}
        <span className="truncate font-semibold">{channel.name}</span>
        <span className="flex min-w-0 flex-1 gap-1 overflow-hidden">
          {channel.agents?.map((a) => (
            <span key={a.cacheKey} className="badge badge-sm badge-ghost">
              <Bot size={12} /> {a.name}
            </span>
          ))}
        </span>
        {channel.canEdit && (
          <button className="btn btn-ghost btn-xs" aria-label={t("apps.chat.channelSettings")} onClick={() => setEditing(true)}>
            <Settings2 size={14} />
          </button>
        )}
      </header>

      <div className="min-h-0 flex-1 overflow-auto p-3" aria-live="polite">
        {page.has && (
          <div className="mb-2 text-center">
            <button className="btn btn-ghost btn-xs" onClick={loadOlder}>
              {t("apps.chat.loadOlder")}
            </button>
          </div>
        )}
        {messages.length + older.length === 0 && <p className="text-center text-sm opacity-60">{t("apps.chat.empty")}</p>}
        {[...older, ...messages].map((m) => (
          <div key={m.cacheKey} className={`chat ${m.fromViewer ? "chat-end" : "chat-start"}`}>
            <div className="chat-header text-xs">
              {m.authorKind === "AGENT" && <Bot size={12} className="inline" />} {m.authorName}
              <time className="ms-1 opacity-50">{time.format(new Date(m.createdAt as string))}</time>
            </div>
            <div
              className={`chat-bubble whitespace-pre-wrap break-words ${
                m.isError ? "chat-bubble-error" : m.fromViewer ? "chat-bubble-primary" : ""
              }`}
            >
              {m.isError ? `${t("apps.chat.agentFailed")} ${m.content}` : m.content}
            </div>
          </div>
        ))}
        {typingNames.length > 0 && (
          <p className="mt-2 text-xs opacity-70">
            <span className="loading loading-dots loading-xs me-1" />
            {t("apps.chat.typing", { names: typingNames.join(", "), count: typingNames.length })}
          </p>
        )}
        <div ref={bottom} />
      </div>

      <form
        className="flex items-end gap-2 border-t border-base-300 p-2"
        onSubmit={(e) => {
          e.preventDefault();
          send(e.currentTarget);
        }}
      >
        <textarea
          name="content"
          rows={2}
          maxLength={16000}
          className="textarea textarea-sm min-h-0 flex-1 resize-none"
          placeholder={t(channel.agents && channel.agents.length > 1 ? "apps.chat.placeholderMention" : "apps.chat.placeholder")}
          aria-label={t("apps.chat.message")}
          onKeyDown={onKeyDown}
        />
        <button className="btn btn-primary btn-sm" disabled={posting} aria-label={t("apps.chat.send")}>
          <Send size={14} />
        </button>
      </form>
      <StatusLine status={status} />
    </>
  );
}
