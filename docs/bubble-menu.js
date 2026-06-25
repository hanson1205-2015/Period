import { gsap } from "https://cdn.jsdelivr.net/npm/gsap@3.15.0/index.js";

(function () {
  const header = document.querySelector("header");
  if (!header) return;

  const nav = header.querySelector("nav");
  const links = nav ? Array.from(nav.querySelectorAll("a")) : [];
  if (!links.length) return;

  const menuItems = links.map((link) => ({
    label: link.textContent.trim(),
    href: link.getAttribute("href"),
    ariaLabel: link.textContent.trim(),
    rotation: Math.random() > 0.5 ? 6 : -6,
    hoverStyles: { bgColor: "var(--accent)", textColor: "#fff" }
  }));

  const currentPath = location.pathname.split("/").pop() || "index.html";

  const logoImg = header.querySelector(".logo img");
  const logoText = header.querySelector(".logo span");

  const menu = document.createElement("nav");
  menu.className = "bubble-menu";
  menu.setAttribute("aria-label", "Main navigation");
  menu.innerHTML = `
    <div class="bubble logo-bubble" aria-label="Logo">
      <span class="logo-content">
        ${logoImg ? `<img src="${logoImg.src}" alt="Logo" class="bubble-logo">` : ""}
        ${logoText ? `<span class="bubble-logo-text">${logoText.textContent}</span>` : ""}
      </span>
    </div>
    <button type="button" class="bubble toggle-bubble menu-btn" aria-label="Toggle menu" aria-pressed="false" aria-expanded="false">
      <span class="menu-line"></span>
      <span class="menu-line short"></span>
    </button>
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

  header.style.display = "none";
  document.body.prepend(menu);
  document.body.appendChild(overlay);

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
