import { createClient, type Sink } from "graphql-ws";
import {
  Environment,
  Network,
  Observable,
  RecordSource,
  Store,
  type FetchFunction,
  type GraphQLResponse,
  type SubscribeFunction,
} from "relay-runtime";

const fetchFn: FetchFunction = async (operation, variables) => {
  const response = await fetch("/graphql", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ query: operation.text, variables }),
  });
  if (!response.ok) throw new Error(`GraphQL request failed: HTTP ${response.status}`);
  return response.json();
};

const ws = createClient({
  url: `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/graphql`,
  lazy: true, // open the socket on the first subscription only
  retryAttempts: Infinity, // the backend restarts on deploys; keep reconnecting
  shouldRetry: () => true,
});

const subscribeFn: SubscribeFunction = (operation, variables) =>
  Observable.create<GraphQLResponse>((sink) =>
    ws.subscribe({ operationName: operation.name, query: operation.text!, variables }, sink as Sink),
  );

export const environment = new Environment({
  network: Network.create(fetchFn, subscribeFn),
  store: new Store(new RecordSource()),
  // Records are identified by the stable `cacheKey`, not by `id`, which is an expiring token (see AGENTS.md).
  getDataID: (fieldValue) => fieldValue.cacheKey ?? fieldValue.id ?? undefined,
});
