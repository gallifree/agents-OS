/** Feedback line shown under a form after submit. */
export type Status = { ok: boolean; text: string } | undefined;

export function StatusLine({ status }: { status: Status }) {
  if (!status) return null;
  return (
    <div role="status" className={`alert alert-soft mt-3 ${status.ok ? "alert-success" : "alert-error"}`}>
      {status.text}
    </div>
  );
}
