/** The `extensions.code` of the first GraphQL error carried by a Relay error, if any. */
export function errorCode(error: unknown): string | undefined {
  const source = (error as { source?: { errors?: { extensions?: { code?: string } }[] } })?.source;
  return source?.errors?.[0]?.extensions?.code;
}
