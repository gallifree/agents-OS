import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { graphql, useLazyLoadQuery, useMutation } from "react-relay";
import { Bot, Copy } from "lucide-react";
import type { AppProps } from "../desktop/apps";
import { errorCode } from "../desktop/errors";
import { StatusLine, type Status } from "../desktop/StatusLine";
import type { AgentsQuery, AgentsQuery$data } from "./__generated__/AgentsQuery.graphql";
import type { AgentsCreateMutation } from "./__generated__/AgentsCreateMutation.graphql";
import type { AgentsUpdateMutation } from "./__generated__/AgentsUpdateMutation.graphql";
import type { AgentsDeleteMutation } from "./__generated__/AgentsDeleteMutation.graphql";
import type { AgentsCreateTokenMutation } from "./__generated__/AgentsCreateTokenMutation.graphql";
import type { AgentsRevokeTokenMutation } from "./__generated__/AgentsRevokeTokenMutation.graphql";

const query = graphql`
  query AgentsQuery {
    agents {
      id
      cacheKey
      name
      model
      systemPrompt
      canEdit
      tokens {
        id
        cacheKey
        name
        lastUsedAt
      }
    }
    availableModels
  }
`;

const createMutation = graphql`
  mutation AgentsCreateMutation($name: String!, $model: String!, $systemPrompt: String!) {
    createAgent(name: $name, model: $model, systemPrompt: $systemPrompt) {
      cacheKey
    }
  }
`;

const updateMutation = graphql`
  mutation AgentsUpdateMutation($id: ID!, $model: String!, $systemPrompt: String!) {
    updateAgent(id: $id, model: $model, systemPrompt: $systemPrompt) {
      cacheKey
      model
      systemPrompt
    }
  }
`;

const deleteMutation = graphql`
  mutation AgentsDeleteMutation($id: ID!) {
    deleteAgent(id: $id)
  }
`;

const createTokenMutation = graphql`
  mutation AgentsCreateTokenMutation($agentId: ID!, $name: String!) {
    createAgentToken(agentId: $agentId, name: $name) {
      token
    }
  }
`;

const revokeTokenMutation = graphql`
  mutation AgentsRevokeTokenMutation($id: ID!) {
    revokeApiKey(id: $id)
  }
`;

type AgentData = AgentsQuery$data["agents"][number];
const errorText = (t: (k: string) => string, err: unknown) => t(`errors.${errorCode(err) ?? "NETWORK"}`);

export default function Agents(_: AppProps) {
  const { t } = useTranslation();
  const [fetchKey, setFetchKey] = useState(0);
  const refresh = () => setFetchKey((k) => k + 1);
  const { agents, availableModels } = useLazyLoadQuery<AgentsQuery>(query, {}, { fetchKey, fetchPolicy: "store-and-network" });
  const [create, creating] = useMutation<AgentsCreateMutation>(createMutation);
  const [status, setStatus] = useState<Status>();

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const formEl = e.currentTarget;
    const form = new FormData(formEl);
    create({
      variables: {
        name: String(form.get("name")),
        model: String(form.get("model")),
        systemPrompt: String(form.get("systemPrompt")),
      },
      onCompleted: () => {
        formEl.reset();
        setStatus({ ok: true, text: t("apps.agents.created") });
        refresh();
      },
      onError: (err) => setStatus({ ok: false, text: errorText(t, err) }),
    });
  };

  return (
    <div className="flex flex-col gap-6 p-4">
      {/* Shared by every model field: suggestions from the enabled providers, free text still allowed. */}
      <datalist id="agent-models">
        {availableModels.map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>

      {agents.length === 0 ? (
        <p className="opacity-70">{t("apps.agents.none")}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {agents.map((a) => (
            <AgentRow key={a.cacheKey} agent={a} onChanged={refresh} />
          ))}
        </ul>
      )}

      <form onSubmit={submit} className="max-w-md">
        <fieldset className="fieldset" disabled={creating}>
          <legend className="fieldset-legend">{t("apps.agents.new")}</legend>
          <label className="fieldset-label" htmlFor="agent-name">
            {t("apps.models.name")}
          </label>
          <input id="agent-name" name="name" className="input w-full" required pattern="[a-z0-9][a-z0-9_\-]{0,31}" autoComplete="off" />
          <p className="text-xs opacity-60">{t("apps.agents.nameHint")}</p>
          <ModelField />
          {availableModels.length === 0 && <p className="text-xs opacity-60">{t("apps.agents.noModels")}</p>}
          <label className="fieldset-label" htmlFor="agent-prompt">
            {t("apps.agents.systemPrompt")}
          </label>
          <textarea id="agent-prompt" name="systemPrompt" className="textarea w-full" rows={4} maxLength={20000} />
          <button className="btn btn-primary btn-sm mt-2 w-fit">{t("apps.settings.create")}</button>
        </fieldset>
        <StatusLine status={status} />
      </form>
    </div>
  );
}

function ModelField({ id = "agent-model", defaultValue }: { id?: string; defaultValue?: string }) {
  const { t } = useTranslation();
  return (
    <>
      <label className="fieldset-label" htmlFor={id}>
        {t("apps.models.model")}
      </label>
      <input
        id={id}
        name="model"
        list="agent-models"
        className="input w-full font-mono"
        required
        placeholder="openai/gpt-4o"
        defaultValue={defaultValue}
        autoComplete="off"
      />
    </>
  );
}

function AgentRow({ agent: a, onChanged }: { agent: AgentData; onChanged: () => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [update, updating] = useMutation<AgentsUpdateMutation>(updateMutation);
  const [remove, removing] = useMutation<AgentsDeleteMutation>(deleteMutation);
  const [createToken, creatingToken] = useMutation<AgentsCreateTokenMutation>(createTokenMutation);
  const [revokeToken] = useMutation<AgentsRevokeTokenMutation>(revokeTokenMutation);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [token, setToken] = useState<string>();
  const [status, setStatus] = useState<Status>();
  const onError = (err: unknown) => setStatus({ ok: false, text: errorText(t, err) });

  const save = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    update({
      variables: { id: a.id, model: String(form.get("model")), systemPrompt: String(form.get("systemPrompt")) },
      onCompleted: () => setStatus({ ok: true, text: t("apps.models.saved") }),
      onError,
    });
  };

  const newToken = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const formEl = e.currentTarget;
    createToken({
      variables: { agentId: a.id, name: String(new FormData(formEl).get("tokenName")) },
      onCompleted: (r) => {
        formEl.reset();
        setToken(r.createAgentToken.token);
        onChanged();
      },
      onError,
    });
  };

  return (
    <li className="rounded-box border border-base-300 p-3">
      <div className="flex items-center gap-2">
        <Bot size={16} />
        <span className="font-medium">@{a.name}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs opacity-60">{a.model}</span>
        {a.canEdit && (
          <button className="btn btn-ghost btn-xs" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
            {t("apps.models.edit")}
          </button>
        )}
      </div>

      {open && (
        <div className="mt-3 flex flex-col gap-4">
          <form onSubmit={save}>
            <fieldset className="fieldset" disabled={updating}>
              <ModelField id={`model-${a.cacheKey}`} defaultValue={a.model} />
              <label className="fieldset-label" htmlFor={`prompt-${a.cacheKey}`}>
                {t("apps.agents.systemPrompt")}
              </label>
              <textarea
                id={`prompt-${a.cacheKey}`}
                name="systemPrompt"
                className="textarea w-full"
                rows={4}
                maxLength={20000}
                defaultValue={a.systemPrompt}
              />
              <div className="mt-2 flex gap-2">
                <button className="btn btn-primary btn-sm">{t("apps.settings.save")}</button>
                <button
                  type="button"
                  className={`btn btn-sm ms-auto ${confirmDelete ? "btn-error" : "btn-ghost"}`}
                  disabled={removing}
                  onBlur={() => setConfirmDelete(false)}
                  onClick={() =>
                    confirmDelete ? remove({ variables: { id: a.id }, onCompleted: onChanged, onError }) : setConfirmDelete(true)
                  }
                >
                  {t(confirmDelete ? "apps.models.confirmDelete" : "apps.models.delete")}
                </button>
              </div>
            </fieldset>
          </form>

          <section>
            <h3 className="text-sm font-semibold">{t("apps.agents.tokens")}</h3>
            <p className="text-xs opacity-60">{t("apps.agents.tokensIntro")}</p>
            {a.tokens.length > 0 && (
              <ul className="mt-2 flex flex-col gap-1 text-sm">
                {a.tokens.map((k) => (
                  <li key={k.cacheKey} className="flex items-center gap-2">
                    <span className="flex-1 truncate">{k.name}</span>
                    <button
                      className="btn btn-ghost btn-xs"
                      onClick={() => revokeToken({ variables: { id: k.id }, onCompleted: onChanged, onError })}
                    >
                      {t("apps.models.revoke")}
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {token && (
              <div role="status" className="alert alert-soft alert-warning mt-2 flex-col items-start">
                <p>{t("apps.models.tokenOnce")}</p>
                <div className="flex w-full items-center gap-2">
                  <code className="min-w-0 flex-1 truncate font-mono text-xs">{token}</code>
                  <button className="btn btn-xs" onClick={() => navigator.clipboard?.writeText(token)}>
                    <Copy size={12} /> {t("apps.models.copy")}
                  </button>
                </div>
              </div>
            )}
            <form onSubmit={newToken} className="mt-2 flex gap-2">
              <input
                name="tokenName"
                className="input input-sm flex-1"
                required
                maxLength={64}
                placeholder={t("apps.agents.tokenName")}
                aria-label={t("apps.agents.tokenName")}
                autoComplete="off"
              />
              <button className="btn btn-sm" disabled={creatingToken}>
                {t("apps.settings.create")}
              </button>
            </form>
          </section>
          <StatusLine status={status} />
        </div>
      )}
    </li>
  );
}
