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
