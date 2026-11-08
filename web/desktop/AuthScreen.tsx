import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { graphql, useMutation } from "react-relay";
import type { AuthScreenLoginMutation } from "./__generated__/AuthScreenLoginMutation.graphql";
import type { AuthScreenSetupMutation } from "./__generated__/AuthScreenSetupMutation.graphql";
import { errorCode } from "./errors";

const loginMutation = graphql`
  mutation AuthScreenLoginMutation($username: String!, $password: String!) {
    login(username: $username, password: $password) {
      cacheKey
    }
  }
`;

const setupMutation = graphql`
  mutation AuthScreenSetupMutation($code: String!, $username: String!, $password: String!) {
    setup(code: $code, username: $username, password: $password) {
      cacheKey
    }
  }
`;

type Props = { setup: boolean; onSignedIn: () => void };

/** Login form, or first-admin creation while the instance has no users. */
export function AuthScreen({ setup, onSignedIn }: Props) {
  const { t } = useTranslation();
  const [login, loggingIn] = useMutation<AuthScreenLoginMutation>(loginMutation);
  const [createAdmin, creating] = useMutation<AuthScreenSetupMutation>(setupMutation);
  const [error, setError] = useState<string>();
  const busy = loggingIn || creating;

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const form = new FormData(e.currentTarget);
    const username = String(form.get("username"));
    const password = String(form.get("password"));
    const callbacks = {
      onCompleted: onSignedIn,
      onError: (err: Error) => setError(t(`errors.${errorCode(err) ?? "NETWORK"}`)),
    };
    setError(undefined);
    if (!setup) return login({ variables: { username, password }, ...callbacks });
    if (password !== form.get("confirm")) return setError(t("errors.PASSWORD_MISMATCH"));
    createAdmin({ variables: { code: String(form.get("code")), username, password }, ...callbacks });
  };

  return (
    <div className="fixed inset-0 grid place-items-center overflow-auto bg-linear-to-br from-base-300 via-base-200 to-base-300 p-4">
      <form onSubmit={submit} className="card w-full max-w-sm bg-base-100 shadow-xl">
        <div className="card-body">
          <h1 className="card-title text-2xl">rust-agents</h1>
          <p className="opacity-70">{t(setup ? "auth.setupIntro" : "auth.loginIntro")}</p>

          <fieldset className="fieldset" disabled={busy}>
            {setup && (
              <>
                <label className="fieldset-label" htmlFor="code">
                  {t("auth.setupCode")}
                </label>
                <input id="code" name="code" className="input w-full font-mono" required autoComplete="off" />
                <p className="text-xs opacity-60">{t("auth.setupCodeHint")}</p>
              </>
            )}
            <label className="fieldset-label" htmlFor="username">
              {t("auth.username")}
            </label>
            <input id="username" name="username" className="input w-full" required autoComplete="username" />
            <label className="fieldset-label" htmlFor="password">
              {t("auth.password")}
            </label>
            <input
              id="password"
              name="password"
              type="password"
              className="input w-full"
              required
              minLength={setup ? 8 : undefined}
              autoComplete={setup ? "new-password" : "current-password"}
            />
            {setup && (
              <>
                <label className="fieldset-label" htmlFor="confirm">
                  {t("auth.confirmPassword")}
                </label>
                <input
                  id="confirm"
                  name="confirm"
                  type="password"
                  className="input w-full"
                  required
                  autoComplete="new-password"
                />
              </>
            )}
          </fieldset>

          {error && (
            <div role="alert" className="alert alert-error alert-soft">
              {error}
            </div>
          )}
          <button className="btn btn-primary mt-2" disabled={busy}>
            {busy && <span className="loading loading-spinner loading-sm" />}
            {t(setup ? "auth.createAdmin" : "auth.signIn")}
          </button>
        </div>
      </form>
    </div>
  );
}
