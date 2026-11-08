import "../i18n";
import { Suspense, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { graphql, RelayEnvironmentProvider, useLazyLoadQuery } from "react-relay";
import { environment } from "../relay/environment";
import type { AppQuery } from "./__generated__/AppQuery.graphql";
import { AuthScreen } from "./AuthScreen";
import { Desktop } from "./Desktop";
import { applyTheme, savedTheme, type Theme } from "./prefs";

applyTheme(savedTheme());

const query = graphql`
  query AppQuery {
    setupRequired
    viewer {
      id
      cacheKey
      username
      isAdmin
      prefs {
        theme
        language
      }
    }
  }
`;

export type Viewer = NonNullable<AppQuery["response"]["viewer"]>;

export function App() {
  return (
    <RelayEnvironmentProvider environment={environment}>
      <Suspense fallback={<FullScreenSpinner />}>
        <Gate />
      </Suspense>
    </RelayEnvironmentProvider>
  );
}

/** Setup screen, login screen or desktop, depending on who is calling. */
function Gate() {
  const [fetchKey, setFetchKey] = useState(0);
  const { setupRequired, viewer } = useLazyLoadQuery<AppQuery>(query, {}, { fetchKey, fetchPolicy: "network-only" });
  const { i18n } = useTranslation();

  // Server-side preferences follow the user across devices.
  useEffect(() => {
    if (viewer?.prefs.theme) applyTheme(viewer.prefs.theme as Theme);
    if (viewer?.prefs.language && viewer.prefs.language !== i18n.language) i18n.changeLanguage(viewer.prefs.language);
  }, [viewer, i18n]);

  if (!viewer) return <AuthScreen setup={setupRequired} onSignedIn={() => setFetchKey((k) => k + 1)} />;
  return <Desktop viewer={viewer} />;
}

function FullScreenSpinner() {
  return (
    <div className="fixed inset-0 grid place-items-center bg-base-200">
      <span className="loading loading-spinner loading-lg" />
    </div>
  );
}
