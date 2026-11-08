import { Suspense, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { fetchQuery, graphql, useLazyLoadQuery, useMutation, useRelayEnvironment } from "react-relay";
import { Copy, KeyRound, Plug, Trash2 } from "lucide-react";
import type { AppProps } from "../desktop/apps";
import { errorCode } from "../desktop/errors";
import { StatusLine, type Status } from "../desktop/StatusLine";
import type { ModelsProvidersQuery } from "./__generated__/ModelsProvidersQuery.graphql";
import type { ModelsProviderModelsQuery } from "./__generated__/ModelsProviderModelsQuery.graphql";
import type { ModelsCreateProviderMutation } from "./__generated__/ModelsCreateProviderMutation.graphql";
import type { ModelsUpdateProviderMutation } from "./__generated__/ModelsUpdateProviderMutation.graphql";
import type { ModelsDeleteProviderMutation } from "./__generated__/ModelsDeleteProviderMutation.graphql";
import type { ModelsApiKeysQuery } from "./__generated__/ModelsApiKeysQuery.graphql";
import type { ModelsCreateApiKeyMutation } from "./__generated__/ModelsCreateApiKeyMutation.graphql";
import type { ModelsRevokeApiKeyMutation } from "./__generated__/ModelsRevokeApiKeyMutation.graphql";
import type { ModelsUsageQuery } from "./__generated__/ModelsUsageQuery.graphql";

const providersQuery = graphql`
  query ModelsProvidersQuery {
    providers {
      id
      cacheKey
      name
      baseUrl
      hasApiKey
      enabled
    }
  }
`;

const providerModelsQuery = graphql`
  query ModelsProviderModelsQuery($id: ID!) {
    providerModels(id: $id)
  }
`;

const createProviderMutation = graphql`
  mutation ModelsCreateProviderMutation($name: String!, $baseUrl: String!, $apiKey: String!) {
    createProvider(name: $name, baseUrl: $baseUrl, apiKey: $apiKey) {
      cacheKey
    }
  }
`;

const updateProviderMutation = graphql`
  mutation ModelsUpdateProviderMutation($id: ID!, $baseUrl: String, $apiKey: String, $enabled: Boolean) {
    updateProvider(id: $id, baseUrl: $baseUrl, apiKey: $apiKey, enabled: $enabled) {
      cacheKey
      baseUrl
      hasApiKey
      enabled
    }
  }
`;

const deleteProviderMutation = graphql`
  mutation ModelsDeleteProviderMutation($id: ID!) {
    deleteProvider(id: $id)
  }
`;

const apiKeysQuery = graphql`
  query ModelsApiKeysQuery {
    apiKeys {
      id
      cacheKey
      name
      createdAt
      lastUsedAt
    }
  }
`;

const createApiKeyMutation = graphql`
  mutation ModelsCreateApiKeyMutation($name: String!) {
    createApiKey(name: $name) {
      token
      apiKey {
        cacheKey
      }
    }
  }
`;

const revokeApiKeyMutation = graphql`
  mutation ModelsRevokeApiKeyMutation($id: ID!) {
    revokeApiKey(id: $id)
  }
`;

const usageQuery = graphql`
  query ModelsUsageQuery($days: Int!) {
    llmUsage(days: $days) {
      provider
      model
      requests
      errors
      promptTokens
      completionTokens
    }
  }
`;

type Tab = "providers" | "keys" | "usage";

export default function Models({ viewer }: AppProps) {
  const { t } = useTranslation();
  const tabs: Tab[] = viewer.isAdmin ? ["providers", "keys", "usage"] : ["keys", "usage"];
  const [tab, setTab] = useState<Tab>(tabs[0]);

  return (
    <div className="p-4">
      <div role="tablist" className="tabs tabs-border mb-4">
        {tabs.map((id) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={`tab ${tab === id ? "tab-active" : ""}`}
            onClick={() => setTab(id)}
          >
            {t(`apps.models.tabs.${id}`)}
          </button>
        ))}
      </div>
      <Suspense fallback={<span className="loading loading-spinner" />}>
        {tab === "providers" && <Providers />}
        {tab === "keys" && <ApiKeys />}
        {tab === "usage" && <Usage />}
      </Suspense>
    </div>
  );
}

const errorText = (t: (k: string) => string, err: unknown) => t(`errors.${errorCode(err) ?? "NETWORK"}`);

// ---- Providers ---------------------------------------------------------------------------------

/** Base URLs of common OpenAI-compatible APIs; "ollama" is the stack's optional inference service. */
const presets: Record<string, string> = {
  openai: "https://api.openai.com/v1",
  anthropic: "https://api.anthropic.com/v1",
  openrouter: "https://openrouter.ai/api/v1",
  ollama: "http://inference:11434/v1",
};

function Providers() {
  const { t } = useTranslation();
  const [fetchKey, setFetchKey] = useState(0);
  const refresh = () => setFetchKey((k) => k + 1);
  const { providers } = useLazyLoadQuery<ModelsProvidersQuery>(
    providersQuery,
    {},
    { fetchKey, fetchPolicy: "store-and-network" },
  );
  const [create, creating] = useMutation<ModelsCreateProviderMutation>(createProviderMutation);
  const [status, setStatus] = useState<Status>();
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const formEl = e.currentTarget;
    create({
      variables: { name, baseUrl, apiKey: String(new FormData(formEl).get("apiKey") ?? "") },
      onCompleted: () => {
        formEl.reset();
        setName("");
        setBaseUrl("");
        setStatus({ ok: true, text: t("apps.models.providerCreated") });
        refresh();
      },
      onError: (err) => setStatus({ ok: false, text: errorText(t, err) }),
    });
  };

  return (
    <div className="flex flex-col gap-6">
      {providers.length === 0 ? (
        <p className="opacity-70">{t("apps.models.noProviders")}</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {providers.map((p) => (
            <ProviderRow key={p.cacheKey} provider={p} onDeleted={refresh} />
          ))}
        </ul>
      )}

      <form onSubmit={submit} className="max-w-md">
        <fieldset className="fieldset" disabled={creating}>
          <legend className="fieldset-legend">{t("apps.models.newProvider")}</legend>
          <div className="flex flex-wrap gap-1">
            {Object.entries(presets).map(([preset, url]) => (
              <button
                key={preset}
                type="button"
                className="btn btn-xs"
                onClick={() => {
                  setName(preset);
                  setBaseUrl(url);
                }}
              >
                {preset}
              </button>
            ))}
          </div>
          <label className="fieldset-label" htmlFor="provider-name">
            {t("apps.models.name")}
          </label>
          <input
            id="provider-name"
            className="input w-full"
            required
            pattern="[a-z0-9][a-z0-9_\-]{0,31}"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
          <p className="text-xs opacity-60">{t("apps.models.nameHint")}</p>
          <label className="fieldset-label" htmlFor="provider-url">
            {t("apps.models.baseUrl")}
          </label>
          <input
            id="provider-url"
            className="input w-full"
            type="url"
            required
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
          />
          <label className="fieldset-label" htmlFor="provider-key">
            {t("apps.models.apiKey")}
          </label>
          <input id="provider-key" name="apiKey" type="password" className="input w-full" autoComplete="off" />
          <p className="text-xs opacity-60">{t("apps.models.apiKeyHint")}</p>
          <button className="btn btn-primary btn-sm mt-2 w-fit" disabled={creating}>
            {t("apps.settings.create")}
          </button>
        </fieldset>
        <StatusLine status={status} />
      </form>
    </div>
  );
}

type ProviderData = ModelsProvidersQuery["response"]["providers"][number];

function ProviderRow({ provider: p, onDeleted }: { provider: ProviderData; onDeleted: () => void }) {
  const { t } = useTranslation();
  const environment = useRelayEnvironment();
  const [update, updating] = useMutation<ModelsUpdateProviderMutation>(updateProviderMutation);
  const [remove, removing] = useMutation<ModelsDeleteProviderMutation>(deleteProviderMutation);
  const [models, setModels] = useState<readonly string[]>();
  const [testing, setTesting] = useState(false);
  const [status, setStatus] = useState<Status>();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [editing, setEditing] = useState(false);

  const test = () => {
    setTesting(true);
    setStatus(undefined);
    fetchQuery<ModelsProviderModelsQuery>(environment, providerModelsQuery, { id: p.id }, { fetchPolicy: "network-only" })
      .toPromise()
      .then((r) => setModels(r?.providerModels ?? []))
      .catch((err) => {
        setModels(undefined);
        setStatus({ ok: false, text: errorText(t, err) });
      })
      .finally(() => setTesting(false));
  };

  const save = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const apiKey = String(form.get("apiKey") ?? "");
    update({
      // An untouched key field keeps the stored key; "clear" sends an empty key, which removes it.
      variables: { id: p.id, baseUrl: String(form.get("baseUrl")), apiKey: form.get("clearKey") ? "" : apiKey || null },
      onCompleted: () => {
        setEditing(false);
        setStatus({ ok: true, text: t("apps.models.saved") });
      },
      onError: (err) => setStatus({ ok: false, text: errorText(t, err) }),
    });
  };

  return (
    <li className="rounded-box border border-base-300 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <input
          type="checkbox"
          className="toggle toggle-sm toggle-success"
          aria-label={t("apps.models.enabled")}
          checked={p.enabled}
          disabled={updating}
          onChange={(e) => update({ variables: { id: p.id, enabled: e.target.checked } })}
        />
        <span className="font-medium">{p.name}</span>
        {p.hasApiKey && (
          <span className="badge badge-sm badge-ghost" title={t("apps.models.hasKey")}>
            <KeyRound size={12} />
          </span>
        )}
        <span className="min-w-0 flex-1 truncate text-xs opacity-60">{p.baseUrl}</span>
        <button className="btn btn-ghost btn-xs" onClick={test} disabled={testing}>
          <Plug size={14} /> {t("apps.models.test")}
        </button>
        <button className="btn btn-ghost btn-xs" onClick={() => setEditing((v) => !v)}>
          {t("apps.models.edit")}
        </button>
        <button
          className={`btn btn-xs ${confirmDelete ? "btn-error" : "btn-ghost"}`}
          disabled={removing}
          aria-label={t("apps.models.delete")}
          onBlur={() => setConfirmDelete(false)}
          onClick={() =>
            confirmDelete
              ? remove({ variables: { id: p.id }, onCompleted: onDeleted, onError: (err) => setStatus({ ok: false, text: errorText(t, err) }) })
              : setConfirmDelete(true)
          }
        >
          <Trash2 size={14} /> {confirmDelete && t("apps.models.confirmDelete")}
        </button>
      </div>

      {editing && (
        <form onSubmit={save} className="mt-3 max-w-md">
          <fieldset className="fieldset" disabled={updating}>
            <label className="fieldset-label" htmlFor={`url-${p.cacheKey}`}>
              {t("apps.models.baseUrl")}
            </label>
            <input id={`url-${p.cacheKey}`} name="baseUrl" type="url" className="input input-sm w-full" required defaultValue={p.baseUrl} />
            <label className="fieldset-label" htmlFor={`key-${p.cacheKey}`}>
              {t("apps.models.replaceKey")}
            </label>
            <input id={`key-${p.cacheKey}`} name="apiKey" type="password" className="input input-sm w-full" autoComplete="off" />
            {p.hasApiKey && (
              <label className="fieldset-label">
                <input name="clearKey" type="checkbox" className="checkbox checkbox-xs" />
                {t("apps.models.clearKey")}
              </label>
            )}
            <button className="btn btn-primary btn-sm mt-2 w-fit">{t("apps.settings.save")}</button>
          </fieldset>
        </form>
      )}

      {models && (
        <div className="mt-2 text-sm">
          <p className="opacity-70">{t("apps.models.modelCount", { count: models.length })}</p>
          <ul className="mt-1 flex max-h-32 flex-wrap gap-1 overflow-auto">
            {models.map((m) => (
              <li key={m} className="badge badge-sm badge-outline font-mono">
                {p.name}/{m}
              </li>
            ))}
          </ul>
        </div>
      )}
      <StatusLine status={status} />
    </li>
  );
}

// ---- API keys ----------------------------------------------------------------------------------

function ApiKeys() {
  const { t, i18n } = useTranslation();
  const [fetchKey, setFetchKey] = useState(0);
  const { apiKeys } = useLazyLoadQuery<ModelsApiKeysQuery>(apiKeysQuery, {}, { fetchKey, fetchPolicy: "store-and-network" });
  const [create, creating] = useMutation<ModelsCreateApiKeyMutation>(createApiKeyMutation);
  const [revoke] = useMutation<ModelsRevokeApiKeyMutation>(revokeApiKeyMutation);
  const [token, setToken] = useState<string>();
  const [status, setStatus] = useState<Status>();
  const date = new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" });

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const formEl = e.currentTarget;
    create({
      variables: { name: String(new FormData(formEl).get("name")) },
      onCompleted: (r) => {
        formEl.reset();
        setStatus(undefined);
        setToken(r.createApiKey.token);
        setFetchKey((k) => k + 1);
      },
      onError: (err) => setStatus({ ok: false, text: errorText(t, err) }),
    });
  };

  const example = token
    ? `curl ${location.origin}/v1/chat/completions \\\n  -H "Authorization: Bearer ${token}" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model": "<provider>/<model>", "messages": [{"role": "user", "content": "Hello"}]}'`
    : "";

  return (
    <div className="flex flex-col gap-6">
      <p className="text-sm opacity-70">{t("apps.models.keysIntro")}</p>
      {apiKeys.length > 0 && (
        <table className="table table-sm">
          <thead>
            <tr>
              <th>{t("apps.models.name")}</th>
              <th>{t("apps.settings.created")}</th>
              <th>{t("apps.models.lastUsed")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {apiKeys.map((k) => (
              <tr key={k.cacheKey}>
                <td className="font-medium">{k.name}</td>
                <td className="opacity-70">{date.format(new Date(k.createdAt as string))}</td>
                <td className="opacity-70">{k.lastUsedAt ? date.format(new Date(k.lastUsedAt as string)) : "—"}</td>
                <td className="text-right">
                  <button
                    className="btn btn-ghost btn-xs"
                    onClick={() => revoke({ variables: { id: k.id }, onCompleted: () => setFetchKey((n) => n + 1) })}
                  >
                    {t("apps.models.revoke")}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {token && (
        <div role="status" className="alert alert-soft alert-warning flex-col items-start">
          <p>{t("apps.models.tokenOnce")}</p>
          <div className="flex w-full items-center gap-2">
            <code className="min-w-0 flex-1 truncate font-mono text-xs">{token}</code>
            <button className="btn btn-xs" onClick={() => navigator.clipboard?.writeText(token)}>
              <Copy size={12} /> {t("apps.models.copy")}
            </button>
          </div>
          <pre className="w-full overflow-x-auto rounded bg-base-200 p-2 text-xs">{example}</pre>
        </div>
      )}

      <form onSubmit={submit} className="max-w-sm">
        <fieldset className="fieldset" disabled={creating}>
          <legend className="fieldset-legend">{t("apps.models.newKey")}</legend>
          <label className="fieldset-label" htmlFor="key-name">
            {t("apps.models.name")}
          </label>
          <input id="key-name" name="name" className="input w-full" required maxLength={64} autoComplete="off" />
          <button className="btn btn-primary btn-sm mt-2 w-fit" disabled={creating}>
            {t("apps.settings.create")}
          </button>
        </fieldset>
        <StatusLine status={status} />
      </form>
    </div>
  );
}

// ---- Usage -------------------------------------------------------------------------------------

const periods = [1, 7, 30];

function Usage() {
  const { t, i18n } = useTranslation();
  const [days, setDays] = useState(30);
  const { llmUsage } = useLazyLoadQuery<ModelsUsageQuery>(usageQuery, { days }, { fetchPolicy: "network-only" });
  const n = new Intl.NumberFormat(i18n.language);

  return (
    <div className="flex flex-col gap-4">
      <div className="join">
        {periods.map((d) => (
          <input
            key={d}
            type="radio"
            name="period"
            className="join-item btn btn-sm"
            aria-label={t("apps.models.lastDays", { count: d })}
            checked={days === d}
            onChange={() => setDays(d)}
          />
        ))}
      </div>
      {llmUsage.length === 0 ? (
        <p className="opacity-70">{t("apps.models.noUsage")}</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="table table-sm">
            <thead>
              <tr>
                <th>{t("apps.models.model")}</th>
                <th className="text-right">{t("apps.models.requests")}</th>
                <th className="text-right">{t("apps.models.errors")}</th>
                <th className="text-right">{t("apps.models.promptTokens")}</th>
                <th className="text-right">{t("apps.models.completionTokens")}</th>
              </tr>
            </thead>
            <tbody>
              {llmUsage.map((u) => (
                <tr key={`${u.provider}/${u.model}`}>
                  <td className="font-mono text-xs">
                    {u.provider}/{u.model}
                  </td>
                  <td className="text-right">{n.format(u.requests)}</td>
                  <td className="text-right">{n.format(u.errors)}</td>
                  <td className="text-right">{n.format(u.promptTokens)}</td>
                  <td className="text-right">{n.format(u.completionTokens)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
