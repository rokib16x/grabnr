import { expect, test, type Page } from "@playwright/test";

/** Open the app past the first-run welcome. */
async function open(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Skip" }).click();
  await expect(page.getByRole("heading", { name: "All Downloads" })).toBeVisible();
}

const row = (page: Page, name: string) => page.locator("li.row", { hasText: name });

test("first run shows the welcome steps and then the download list", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("dialog", { name: "Welcome to grabnr" })).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(page.getByText("Your connections")).toBeVisible();
  await expect(page.getByText("3 connections ready.")).toBeVisible();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Continue" }).click();
  await page.getByRole("button", { name: "Start using grabnr" }).click();
  await expect(row(page, "Beautiful Nature 4K.mp4")).toBeVisible();
});

test("the sidebar lists connections with their speeds", async ({ page }) => {
  await open(page);
  const nav = page.getByRole("navigation", { name: "Sidebar" });
  for (const name of ["Wi-Fi", "Ethernet", "iPhone Hotspot"]) await expect(nav.getByText(name)).toBeVisible();
});

test("adding a download from the dialog puts it in the list", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Add download" }).click();
  await page.getByLabel("Links").fill("https://files.example.com/new-release.zip");
  await page.getByRole("button", { name: "Add Download", exact: true }).last().click();
  await expect(row(page, "new-release.zip")).toBeVisible();
});

test("pasting several links adds them all", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Add download" }).click();
  await page.getByLabel("Links").fill("https://a.example/one.zip\nhttps://a.example/two.zip\nhttps://a.example/three.zip");
  await page.getByRole("button", { name: "Add 3 Downloads" }).click();
  for (const n of ["one.zip", "two.zip", "three.zip"]) await expect(row(page, n)).toBeVisible();
});

test("a link found on a page can be picked and added", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Add download" }).click();
  await page.getByLabel("Links").fill("https://example.com/files/index.html");
  await page.getByRole("button", { name: /List the links on this page/ }).click();
  await page.getByLabel("Filter links").fill(".pdf");
  await page.getByRole("checkbox").first().check();
  await page.getByRole("button", { name: "Add 1 Download" }).click();
  await expect(row(page, "manual.pdf")).toBeVisible();
});

test("pausing and resuming a download changes its state", async ({ page }) => {
  await open(page);
  const r = row(page, "Figma Setup.dmg");
  await r.getByRole("button", { name: "Pause" }).click();
  await expect(r).toContainText("Paused");
  await r.getByRole("button", { name: "Resume" }).click();
  await expect(r).not.toContainText("Paused");
});

test("categories and search narrow the list", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: /^Video/ }).click();
  await expect(row(page, "Beautiful Nature 4K.mp4")).toBeVisible();
  await expect(row(page, "Figma Setup.dmg")).toHaveCount(0);
  await page.getByRole("button", { name: /^All Downloads/ }).click();
  await page.getByLabel("Search downloads").fill("lo-fi");
  await expect(page.locator("li.row")).toHaveCount(1);
  await expect(row(page, "Lo-fi Mix.mp3")).toBeVisible();
});

test("history shows totals and the share of bytes per connection", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: /^History/ }).click();
  await expect(page.getByText("Time saved (estimate)")).toBeVisible();
  await expect(page.getByRole("img", { name: "Share of bytes per connection" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Export CSV" })).toBeEnabled();
});

test("the command palette finds actions and downloads", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Control+k");
  const box = page.getByLabel("Command", { exact: true });
  await box.fill("figma");
  await expect(page.getByRole("option", { name: /Figma Setup\.dmg/ })).toBeVisible();
  await box.fill("pause all");
  await page.keyboard.press("Enter");
  await expect(row(page, "Figma Setup.dmg")).toContainText("Paused");
});

test("a schedule rule can be added, edited and removed", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Manage Connections" }).click();
  await page.getByRole("button", { name: "Add rule" }).click();
  const rule = page.locator(".rule").first();
  await expect(rule).toBeVisible();
  await rule.getByLabel("Rule name").fill("Night");
  await rule.getByLabel("What to do").selectOption("pause");
  await expect(rule.getByLabel("Speed limit in MB/s")).toHaveCount(0);
  await rule.getByRole("button", { name: "Sat" }).click();
  await expect(rule.getByRole("button", { name: "Sat" })).toHaveAttribute("aria-pressed", "true");
  await rule.getByRole("button", { name: "Delete" }).click();
  await expect(page.locator(".rule")).toHaveCount(0);
});

test("a stopped download can be pointed at a new link", async ({ page }) => {
  await open(page);
  const r = row(page, "Project Files.zip");
  await r.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: "Change link…" }).click();
  await page.getByLabel("New link").fill("https://cdn.example.com/Project%20Files.zip?sig=fresh");
  await page.getByRole("button", { name: "Change and resume" }).click();
  await expect(r).not.toContainText("Paused");
});

test("queue order can be changed by dragging", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "More" }).first().click();
  await page.getByRole("menuitem", { name: /Queue order/ }).click();
  const names = async () => page.locator("li.row .name").allTextContents();
  const before = await names();
  const from = before.indexOf("Beautiful Nature 4K.mp4");
  const to = before.indexOf("Project Files.zip");
  expect(from).toBeGreaterThan(to);
  await row(page, "Beautiful Nature 4K.mp4").dragTo(row(page, "Project Files.zip"));
  await expect.poll(async () => (await names()).indexOf("Beautiful Nature 4K.mp4")).toBeLessThan((await names()).indexOf("Project Files.zip") + 1);
  expect((await names()).indexOf("Beautiful Nature 4K.mp4")).toBeLessThan((await names()).indexOf("Project Files.zip"));
});

test("the menu bar popover shows active downloads and actions", async ({ page }) => {
  await page.goto("/?view=tray");
  await expect(page.getByText("grabnr", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /Show All Downloads/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Preferences/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Quit grabnr/ })).toBeVisible();
});

test("a streaming link offers a choice of quality", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Add download" }).click();
  await page.getByLabel("Links").fill("https://v.example/master.m3u8");
  await page.getByRole("button", { name: /Choose quality/ }).click();
  const q = page.getByLabel("Quality");
  await expect(q).toBeVisible();
  await q.selectOption({ label: "720p · 2.8 Mbps" });
  await page.getByRole("button", { name: "Add Download", exact: true }).last().click();
  await expect(row(page, "master.m3u8")).toBeVisible();
});
