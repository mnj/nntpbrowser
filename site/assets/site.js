"use strict";

const views = {
  gallery: ["gallery.png", "Media gallery with temporary previews, adjustable tile size and cached-group date filtering", "Gallery · Adjustable tiles, cached-group date filters, and previews that stay in RAM until you save."],
  articles: ["articles.png", "Mail-style article list and details pane with selected files and download controls", "Articles · Search cached titles, select files, and read the active article in the details pane."],
  downloads: ["downloads.png", "Download transfers with overall progress, per-job stages and direct icon actions", "Downloads · Pause, resume, cancel, and retry. Track transfers separately from completed files."],
  recovery: ["download-recovery.png", "Download jobs finding missing parts, downloading repair data and verifying files", "Recovery · Find missing parts, fetch repair data as needed, then verify the files before completion."],
};
const tabs = [...document.querySelectorAll("[data-view]")];
const panel = document.getElementById("tour-panel");
const image = document.getElementById("tour-image");
const link = document.getElementById("tour-link");
const caption = document.getElementById("tour-caption");

function selectView(tab) {
  const view = views[tab.dataset.view];
  tabs.forEach((item) => {
    const active = item === tab;
    item.setAttribute("aria-selected", String(active));
    item.tabIndex = active ? 0 : -1;
  });
  image.src = "assets/screenshots/" + view[0];
  image.alt = view[1];
  link.href = image.src;
  caption.textContent = view[2];
  panel.setAttribute("aria-labelledby", tab.id);
}

tabs.forEach((tab, index) => {
  tab.addEventListener("click", () => selectView(tab));
  tab.addEventListener("keydown", (event) => {
    let next;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    if (event.key === "ArrowLeft") next = (index + tabs.length - 1) % tabs.length;
    if (event.key === "Home") next = 0;
    if (event.key === "End") next = tabs.length - 1;
    if (next === undefined) return;
    event.preventDefault();
    selectView(tabs[next]);
    tabs[next].focus();
  });
});

const dialog = document.getElementById("lightbox");
const fullImage = document.getElementById("lightbox-image");
document.querySelectorAll("[data-lightbox]").forEach((anchor) => {
  anchor.addEventListener("click", (event) => {
    if (event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    if (typeof dialog.showModal !== "function") return;
    event.preventDefault();
    const thumbnail = anchor.querySelector("img");
    fullImage.src = anchor.href;
    fullImage.alt = thumbnail.alt;
    document.getElementById("lightbox-title").textContent = thumbnail.alt;
    dialog.showModal();
  });
});
document.getElementById("lightbox-close").addEventListener("click", () => dialog.close());
dialog.addEventListener("click", (event) => {
  if (event.target !== dialog) return;
  const bounds = dialog.getBoundingClientRect();
  if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) dialog.close();
});
