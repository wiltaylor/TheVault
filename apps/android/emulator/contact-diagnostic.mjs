/** Diagnose a slow contact without changing the app's activity state. */
export async function contactDiagnostic(find, wait, snapshot) {
  try {
    return await wait("a person for the contact (no nudge)", find, { timeout: 30_000, every: 1_000 });
  } catch {
    snapshot();
    return wait("a person for the contact (still no nudge)", find, { timeout: 90_000, every: 1_000 });
  }
}
