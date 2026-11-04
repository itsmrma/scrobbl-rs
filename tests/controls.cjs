// Exercise the same custom popup and options as a person, including portals.
async function chooseOption(control, value) {
  control = control.and(control.page().getByRole("combobox"));
  await control.click();
  await control
    .page()
    .locator('[role="listbox"]:visible')
    .waitFor({ state: "visible", timeout: 5000 });
  const id = await control.getAttribute("aria-controls");
  const listbox = id
    ? control.page().locator(`[id="${id}"]`)
    : control.page().locator('[role="listbox"]:visible');

  if (typeof value === "object") {
    await listbox
      .getByRole("option", { name: value.label, exact: true })
      .click();
    return;
  }
  const byValue = listbox.locator(`[role="option"][data-value="${value}"]`);
  if (await byValue.count()) {
    await byValue.click();
    return;
  }
  const byName = listbox.getByRole("option", { name: String(value) });
  if (await byName.count()) {
    await byName.click();
    return;
  }
  for (const option of await listbox.locator('[role="option"]').all()) {
    if ((await option.getAttribute("data-value")) === String(value)) {
      await option.click();
      return;
    }
  }
  throw new Error(`No option ${value}`);
}
// A button inside native details can be visible before the card stops growing.
// WebKit can finish its scroll/click while that growth moves the hit target.
async function settleDisclosure(disclosure) {
  await disclosure.evaluate((element) =>
    Promise.all(
      element
        .getAnimations({ subtree: true })
        .filter((animation) =>
          Number.isFinite(animation.effect?.getTiming().iterations),
        )
        .map((animation) => animation.finished.catch(() => {})),
    ),
  );
}
module.exports = { chooseOption, settleDisclosure };
