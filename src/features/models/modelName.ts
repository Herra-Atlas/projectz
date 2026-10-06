/** Model ids are often namespaced as `vendor/model:variant`. A label has room for
    the model itself, not the routing prefix, so `stepfun/step-3.7-flash:free`
    shows as `step-3.7-flash:free`. Casing is left exactly as the provider reports
    it, and the full id stays on the title attribute and is what gets sent and
    stored in favorites.
 *
 * One definition, so the chat picker's label and the statistics breakdown cannot
 * drift into showing the same model two ways. */
export function displayModelName(model: string): string {
  return model.includes("/") ? model.slice(model.lastIndexOf("/") + 1) : model;
}