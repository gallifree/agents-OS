import { Suspense, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { graphql, useLazyLoadQuery, useMutation } from "react-relay";
import { languages, type Language } from "../i18n";
import type { AppProps } from "../desktop/apps";
import { errorCode } from "../desktop/errors";
import { applyTheme, savedTheme, type Theme } from "../desktop/prefs";
import type { SettingsPrefsMutation } from "./__generated__/SettingsPrefsMutation.graphql";
import type { SettingsPasswordMutation } from "./__generated__/SettingsPasswordMutation.graphql";
import type { SettingsUsersQuery } from "./__generated__/SettingsUsersQuery.graphql";
import type { SettingsCreateUserMutation } from "./__generated__/SettingsCreateUserMutation.graphql";

const prefsMutation = graphql`
  mutation SettingsPrefsMutation($theme: String, $language: String) {
    updatePrefs(theme: $theme, language: $language) {
      cacheKey
      prefs {
        theme
        language
      }
    }
  }
`;

const passwordMutation = graphql`
  mutation SettingsPasswordMutation($current: String!, $new: String!) {
    changePassword(current: $current, new: $new)
  }
`;

const usersQuery = graphql`
  query SettingsUsersQuery {
    users(first: 100) {
      edges {
        node {
          cacheKey
          username
          isAdmin
          createdAt
        }
      }
    }
  }
`;

const createUserMutation = graphql`
  mutation SettingsCreateUserMutation($username: String!, $password: String!, $isAdmin: Boolean!) {
    createUser(username: $username, password: $password, isAdmin: $isAdmin) {
      cacheKey
    }
  }
`;

type Tab = "appearance" | "account" | "users";

export default function Settings({ viewer }: AppProps) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<Tab>("appearance");
  const tabs: Tab[] = viewer.isAdmin ? ["appearance", "account", "users"] : ["appearance", "account"];

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
            {t(`apps.settings.tabs.${id}`)}
          </button>
        ))}
      </div>
      {tab === "appearance" && <Appearance />}
      {tab === "account" && <Account />}
      {tab === "users" && (
        <Suspense fallback={<span className="loading loading-spinner" />}>
          <Users />
        </Suspense>
      )}
    </div>
  );
}

const themes: Theme[] = ["system", "light", "dark"];

function Appearance() {
  const { t, i18n } = useTranslation();
  const [theme, setTheme] = useState(savedTheme);
  const [save] = useMutation<SettingsPrefsMutation>(prefsMutation);

  return (
    <>
      <fieldset className="fieldset">
        <legend className="fieldset-legend">{t("apps.settings.theme")}</legend>
        <div className="join">
          {themes.map((th) => (
            <input
              key={th}
              type="radio"
              name="theme"
              className="join-item btn btn-sm"
              aria-label={t(`apps.settings.themes.${th}`)}
              checked={theme === th}
              onChange={() => {
                setTheme(th);
                applyTheme(th);
                save({ variables: { theme: th } });
              }}
            />
          ))}
        </div>
      </fieldset>

      <fieldset className="fieldset mt-4">
        <legend className="fieldset-legend">{t("apps.settings.language")}</legend>
        <select
          className="select select-sm w-48"
          value={i18n.resolvedLanguage}
          onChange={(e) => {
            const language = e.target.value as Language;
            i18n.changeLanguage(language);
            save({ variables: { language } });
          }}
        >
          {Object.entries(languages).map(([code, name]) => (
            <option key={code} value={code}>
              {name}
            </option>
          ))}
        </select>
      </fieldset>
    </>
  );
}

/** Feedback line shown under a form after submit. */
type Status = { ok: boolean; text: string } | undefined;

function StatusLine({ status }: { status: Status }) {
  if (!status) return null;
  return (
    <div role="status" className={`alert alert-soft mt-3 ${status.ok ? "alert-success" : "alert-error"}`}>
      {status.text}
    </div>
  );
}

function Account() {
  const { t } = useTranslation();
  const [change, saving] = useMutation<SettingsPasswordMutation>(passwordMutation);
  const [status, setStatus] = useState<Status>();

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const formEl = e.currentTarget;
    const form = new FormData(formEl);
    if (form.get("new") !== form.get("confirm")) return setStatus({ ok: false, text: t("errors.PASSWORD_MISMATCH") });
    change({
      variables: { current: String(form.get("current")), new: String(form.get("new")) },
      onCompleted: () => {
        formEl.reset();
        setStatus({ ok: true, text: t("apps.settings.passwordChanged") });
      },
      onError: (err) => setStatus({ ok: false, text: t(`errors.${errorCode(err) ?? "NETWORK"}`) }),
    });
  };

  return (
    <form onSubmit={submit} className="max-w-sm">
      <fieldset className="fieldset" disabled={saving}>
        <legend className="fieldset-legend">{t("apps.settings.changePassword")}</legend>
        <label className="fieldset-label" htmlFor="current">
          {t("apps.settings.currentPassword")}
        </label>
        <input id="current" name="current" type="password" className="input w-full" required autoComplete="current-password" />
        <label className="fieldset-label" htmlFor="new">
          {t("apps.settings.newPassword")}
        </label>
        <input id="new" name="new" type="password" className="input w-full" required minLength={8} autoComplete="new-password" />
        <label className="fieldset-label" htmlFor="confirm">
          {t("auth.confirmPassword")}
        </label>
        <input id="confirm" name="confirm" type="password" className="input w-full" required autoComplete="new-password" />
        <p className="text-xs opacity-60">{t("apps.settings.otherSessionsSignedOut")}</p>
        <button className="btn btn-primary btn-sm mt-2 w-fit" disabled={saving}>
          {t("apps.settings.save")}
        </button>
      </fieldset>
      <StatusLine status={status} />
    </form>
  );
}

function Users() {
  const { t, i18n } = useTranslation();
  const [fetchKey, setFetchKey] = useState(0);
  const { users } = useLazyLoadQuery<SettingsUsersQuery>(usersQuery, {}, { fetchKey, fetchPolicy: "store-and-network" });
  const [create, creating] = useMutation<SettingsCreateUserMutation>(createUserMutation);
  const [status, setStatus] = useState<Status>();
  const date = new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium" });

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const formEl = e.currentTarget;
    const form = new FormData(formEl);
    create({
      variables: {
        username: String(form.get("username")),
        password: String(form.get("password")),
        isAdmin: form.get("isAdmin") === "on",
      },
      onCompleted: () => {
        formEl.reset();
        setStatus({ ok: true, text: t("apps.settings.userCreated") });
        setFetchKey((k) => k + 1);
      },
      onError: (err) => setStatus({ ok: false, text: t(`errors.${errorCode(err) ?? "NETWORK"}`) }),
    });
  };

  return (
    <div className="flex flex-col gap-6">
      <table className="table table-sm">
        <thead>
          <tr>
            <th>{t("auth.username")}</th>
            <th>{t("apps.settings.role")}</th>
            <th>{t("apps.settings.created")}</th>
          </tr>
        </thead>
        <tbody>
          {users.edges.map(({ node }) => (
            <tr key={node.cacheKey}>
              <td className="font-medium">{node.username}</td>
              <td>
                {node.isAdmin && <span className="badge badge-sm badge-primary">{t("apps.settings.admin")}</span>}
              </td>
              <td className="opacity-70">{date.format(new Date(node.createdAt as string))}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <form onSubmit={submit} className="max-w-sm">
        <fieldset className="fieldset" disabled={creating}>
          <legend className="fieldset-legend">{t("apps.settings.newUser")}</legend>
          <label className="fieldset-label" htmlFor="new-username">
            {t("auth.username")}
          </label>
          <input id="new-username" name="username" className="input w-full" required autoComplete="off" />
          <label className="fieldset-label" htmlFor="new-user-password">
            {t("auth.password")}
          </label>
          <input
            id="new-user-password"
            name="password"
            type="password"
            className="input w-full"
            required
            minLength={8}
            autoComplete="new-password"
          />
          <label className="fieldset-label mt-2">
            <input name="isAdmin" type="checkbox" className="checkbox checkbox-sm" />
            {t("apps.settings.admin")}
          </label>
          <button className="btn btn-primary btn-sm mt-2 w-fit" disabled={creating}>
            {t("apps.settings.create")}
          </button>
        </fieldset>
        <StatusLine status={status} />
      </form>
    </div>
  );
}
