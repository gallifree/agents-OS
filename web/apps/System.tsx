import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { graphql, useLazyLoadQuery, useMutation, useSubscription } from "react-relay";
import { RefreshCw } from "lucide-react";
import type { SystemQuery } from "./__generated__/SystemQuery.graphql";
import type { SystemPingMutation } from "./__generated__/SystemPingMutation.graphql";
import type { SystemPingsSubscription } from "./__generated__/SystemPingsSubscription.graphql";

const query = graphql`
  query SystemQuery {
    systemStatus {
      name
      ok
      optional
      version
      latencyMs
      error
    }
  }
`;

const pingMutation = graphql`
  mutation SystemPingMutation($message: String!) {
    ping(message: $message)
  }
`;

const pingsSubscription = graphql`
  subscription SystemPingsSubscription {
    pings {
      message
    }
  }
`;

export default function System() {
  const { t } = useTranslation();
  const [fetchKey, setFetchKey] = useState(0);
  const { systemStatus } = useLazyLoadQuery<SystemQuery>(query, {}, { fetchKey, fetchPolicy: "network-only" });

  return (
    <div className="flex flex-col gap-6 p-4">
      <section>
        <div className="mb-2 flex items-center justify-between">
          <h2 className="font-semibold">{t("apps.system.services")}</h2>
          <button className="btn btn-ghost btn-sm" onClick={() => setFetchKey((k) => k + 1)}>
            <RefreshCw size={14} /> {t("apps.system.refresh")}
          </button>
        </div>
        <table className="table table-sm">
          <tbody>
            {systemStatus.map((s) => (
              <tr key={s.name} title={s.error ?? undefined}>
                <td className="font-medium">{s.name}</td>
                <td>
                  <span
                    className={`badge badge-sm ${s.ok ? "badge-success" : s.optional ? "badge-ghost" : "badge-error"}`}
                  >
                    {t(s.ok ? "apps.system.up" : s.optional ? "apps.system.absent" : "apps.system.down")}
                  </span>
                </td>
                <td className="max-w-48 truncate opacity-70">{s.version ?? (s.ok ? "" : s.error)}</td>
                <td className="text-right tabular-nums opacity-70">{s.latencyMs} ms</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <PingTest />
    </div>
  );
}

/** Round trip: mutation → Valkey PUBLISH → subscription over WebSocket. */
function PingTest() {
  const { t } = useTranslation();
  const [commit, sending] = useMutation<SystemPingMutation>(pingMutation);
  const sentAt = useRef(new Map<string, number>());
  const [received, setReceived] = useState<{ message: string; ms?: number }[]>([]);

  useSubscription<SystemPingsSubscription>(
    useMemo(
      () => ({
        subscription: pingsSubscription,
        variables: {},
        onNext: (data) => {
          if (!data) return;
          const { message } = data.pings;
          const start = sentAt.current.get(message);
          sentAt.current.delete(message);
          setReceived((r) => [{ message, ms: start && Math.round(performance.now() - start) }, ...r].slice(0, 5));
        },
      }),
      [],
    ),
  );

  const send = () => {
    const message = `ping-${crypto.randomUUID().slice(0, 8)}`;
    sentAt.current.set(message, performance.now());
    commit({ variables: { message } });
  };

  return (
    <section>
      <div className="mb-2 flex items-center justify-between">
        <h2 className="font-semibold">{t("apps.system.realtime")}</h2>
        <button className="btn btn-primary btn-sm" onClick={send} disabled={sending}>
          {t("apps.system.sendPing")}
        </button>
      </div>
      {received.length === 0 ? (
        <p className="text-sm opacity-60">{t("apps.system.noPings")}</p>
      ) : (
        <ul className="text-sm">
          {received.map((r, i) => (
            <li key={i} className="flex justify-between font-mono">
              <span>{r.message}</span>
              {r.ms !== undefined && <span className="opacity-70">{r.ms} ms</span>}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
