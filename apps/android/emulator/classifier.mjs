// Check 58's server fixtures and phone assertions. No model or credentials are needed.
export const classifierNotes = [
  { type: "email", fields: { title: "Classifier email check 58", scam_probability: 0.91, communication_category: "scam" } },
  { type: "message", fields: { title: "Classifier message check 58", channel: "sms", direction: "in", scam_probability: 0.12, communication_category: "personal" } },
];

export function classifierSynced(response, fields) {
  if (response.status !== 200) return false;
  const actual = JSON.parse(response.body).note?.fields;
  return actual?.scam_probability === fields.scam_probability && actual?.communication_category === fields.communication_category;
}

export async function assertClassifierView(page, fields) {
  for (const [name, selector] of [["scam_probability", "input"], ["communication_category", "select"]]) {
    const control = page.getByTestId(`field-${name}`).locator(selector);
    await control.waitFor({ state: "visible", timeout: 20_000 });
    const actual = await control.inputValue();
    if (actual !== String(fields[name])) throw new Error(`${name}: phone template shows ${actual}, expected ${fields[name]}`);
  }
}
