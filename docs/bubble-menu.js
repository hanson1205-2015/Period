import { gsap } from "https://cdn.jsdelivr.net/npm/gsap@3.15.0/index.js";

const SIDEBAR_OPEN_ICON = `<svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="8" y1="6" x2="21" y2="6"></line><line x1="8" y1="12" x2="21" y2="12"></line><line x1="8" y1="18" x2="21" y2="18"></line><line x1="3" y1="6" x2="3.01" y2="6"></line><line x1="3" y1="12" x2="3.01" y2="12"></line><line x1="3" y1="18" x2="3.01" y2="18"></line></svg>`;

const SIDEBAR_CLOSE_ICON = `<svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18"></line><line x1="6" y1="6" x2="18" y2="18"></line></svg>`;

(function () {
  const menuItems = [
    { label: "Home", href: "index.html" },
    { label: "Docs", href: "docs.html" },
    { label: "Examples", href: "examples.html" },
    { label: "About", href: "about.html" },
  ].map((item) => ({
    ...item,
    ariaLabel: item.label,
    rotation: Math.random() > 0.5 ? 6 : -6,
    hoverStyles: { bgColor: "var(--accent)", textColor: "#fff" },
  }));

  const currentPath = location.pathname.split("/").pop() || "index.html";
  const isHome = currentPath === "index.html" || currentPath === "";
  const isDocs = currentPath === "docs.html";

  const menu = document.createElement("nav");
  menu.className = "bubble-menu" + (isHome ? " home-page" : "") + (isDocs ? " has-sidebar" : "");
  menu.setAttribute("aria-label", "Main navigation");
  menu.innerHTML = `
    <div class="bubble logo-bubble" aria-label="Logo">
      <span class="logo-content">
        <img src="period.svg" alt="Logo" class="bubble-logo">
        <span class="bubble-logo-text">Period</span>
      </span>
    </div>
    <div class="bubble-actions">
      <button type="button" class="bubble toggle-bubble menu-btn" aria-label="Toggle menu" aria-pressed="false" aria-expanded="false">
        <span class="menu-line"></span>
        <span class="menu-line short"></span>
      </button>
    </div>
  `;

  const overlay = document.createElement("div");
  overlay.className = "bubble-menu-items";
  overlay.setAttribute("aria-hidden", "true");
  overlay.innerHTML = `
    <ul class="pill-list" role="menu" aria-label="Menu links">
      ${menuItems
        .map(
          (item, idx) => `
        <li role="none" class="pill-col">
          <a
            role="menuitem"
            href="${item.href}"
            aria-label="${item.ariaLabel}"
            class="pill-link ${item.href.includes(currentPath) ? "active" : ""}"
            style="--item-rot: ${item.rotation}deg; --hover-bg: ${item.hoverStyles.bgColor}; --hover-color: ${item.hoverStyles.textColor}"
            data-index="${idx}"
          >
            <span class="pill-label">${item.label}</span>
          </a>
        </li>
      `
        )
        .join("")}
    </ul>
  `;

  document.body.prepend(menu);
  document.body.appendChild(overlay);

  let sidebarOpen = false;
  let sidebarToggle = null;

  // Sidebar toggle for docs pages; desktop hides it via CSS.
  if (isDocs) {
    sidebarToggle = document.createElement("button");
    sidebarToggle.type = "button";
    sidebarToggle.className = "bubble sidebar-toggle-bubble";
    sidebarToggle.setAttribute("aria-label", "Toggle documentation sidebar");
    sidebarToggle.setAttribute("aria-pressed", "false");
    sidebarToggle.setAttribute("aria-expanded", "false");
    sidebarToggle.innerHTML = `<span class="sidebar-toggle-icon">${SIDEBAR_OPEN_ICON}</span>`;
    menu.querySelector(".bubble-actions").appendChild(sidebarToggle);

    const updateSidebarToggle = () => {
      sidebarToggle.setAttribute("aria-pressed", sidebarOpen ? "true" : "false");
      sidebarToggle.setAttribute("aria-expanded", sidebarOpen ? "true" : "false");
      sidebarToggle.setAttribute(
        "aria-label",
        sidebarOpen ? "Close documentation sidebar" : "Toggle documentation sidebar"
      );
      sidebarToggle.innerHTML = `<span class="sidebar-toggle-icon">${sidebarOpen ? SIDEBAR_CLOSE_ICON : SIDEBAR_OPEN_ICON}</span>`;
    };

    const closeSidebar = () => {
      if (!sidebarOpen) return;
      sidebarOpen = false;
      document.body.classList.remove("sidebar-open");
      updateSidebarToggle();
    };

    sidebarToggle.addEventListener("click", () => {
      sidebarOpen = !sidebarOpen;
      document.body.classList.toggle("sidebar-open", sidebarOpen);
      updateSidebarToggle();
    });

    const sidebarOverlay = document.querySelector(".sidebar-overlay");
    if (sidebarOverlay) {
      sidebarOverlay.addEventListener("click", closeSidebar);
    }
    document.querySelectorAll(".sidebar a").forEach((link) => {
      link.addEventListener("click", closeSidebar);
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && sidebarOpen) closeSidebar();
    });
  }

  const toggle = menu.querySelector(".menu-btn");
  const bubbles = Array.from(overlay.querySelectorAll(".pill-link"));
  const labels = Array.from(overlay.querySelectorAll(".pill-label"));
  let isOpen = false;

  const open = () => {
    isOpen = true;
    toggle.classList.add("open");
    toggle.setAttribute("aria-pressed", "true");
    toggle.setAttribute("aria-expanded", "true");
    overlay.setAttribute("aria-hidden", "false");
    overlay.style.display = "flex";

    gsap.killTweensOf([...bubbles, ...labels]);
    gsap.set(bubbles, { scale: 0, transformOrigin: "50% 50%" });
    gsap.set(labels, { y: 24, autoAlpha: 0 });

    bubbles.forEach((bubble, i) => {
      const delay = i * 0.1 + gsap.utils.random(-0.03, 0.03);
      const tl = gsap.timeline({ delay });
      tl.to(bubble, { scale: 1, duration: 0.5, ease: "back.out(1.5)" });
      tl.to(
        labels[i],
        { y: 0, autoAlpha: 1, duration: 0.5, ease: "power3.out" },
        "-=0.45"
      );
    });
  };

  const close = () => {
    if (!isOpen) return;
    isOpen = false;
    toggle.classList.remove("open");
    toggle.setAttribute("aria-pressed", "false");
    toggle.setAttribute("aria-expanded", "false");
    overlay.setAttribute("aria-hidden", "true");

    gsap.killTweensOf([...bubbles, ...labels]);
    gsap.to(labels, { y: 24, autoAlpha: 0, duration: 0.2, ease: "power3.in" });
    gsap.to(bubbles, {
      scale: 0,
      duration: 0.2,
      ease: "power3.in",
      onComplete: () => {
        overlay.style.display = "none";
      }
    });
  };

  toggle.addEventListener("click", () => {
    isOpen ? close() : open();
  });

  bubbles.forEach((bubble) => {
    bubble.addEventListener("click", () => {
      close();
    });
  });

  overlay.addEventListener("click", (e) => {
    if (e.target === overlay || e.target.closest(".pill-list")) {
      // do nothing on pill clicks; close handled above
    }
    if (e.target === overlay) close();
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && isOpen) close();
  });
})();
