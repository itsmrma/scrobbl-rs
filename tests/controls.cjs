// Exercise the same custom popup and options as a person, including portals.
async function chooseOption(control, value) {
  control = control.and(control.page().getByRole("combobox"));
  await control.click();
  const id = await control.getAttribute("aria-controls");
  const options = control.page().locator(`[id="${id}"] [role="option"]`);
  if (typeof value === "object") {
    await control
      .page()
      .locator(`[id="${id}"]`)
      .getByRole("option", { name: value.label, exact: true })
      .click();
    return;
  }
  for (const option of await options.all()) {
    if ((await option.getAttribute("data-value")) === String(value)) {
      await option.click();
      return;
    }
  }
  throw new Error(`No option ${value}`);
}
module.exports = { chooseOption };
