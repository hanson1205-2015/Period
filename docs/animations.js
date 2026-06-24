// Register GSAP plugins
gsap.registerPlugin(ScrollTrigger);

// Entrance animations on page load
gsap.from("header", {
  y: -30,
  opacity: 0,
  duration: 0.8,
  ease: "power3.out"
});

gsap.from("nav a", {
  y: -10,
  opacity: 0,
  duration: 0.5,
  stagger: 0.08,
  delay: 0.3,
  ease: "power2.out"
});

// Hero section: split title into characters and animate
const heroTitle = document.getElementById("hero-title");
if (heroTitle) {
  const text = heroTitle.textContent;
  heroTitle.innerHTML = text
    .split("")
    .map((char) => `<span class="char">${char === " " ? "&nbsp;" : char}</span>`)
    .join("");

  gsap.from("#hero-title .char", {
    y: 80,
    opacity: 0,
    rotationX: -90,
    duration: 0.9,
    stagger: 0.06,
    delay: 0.2,
    ease: "back.out(1.7)"
  });
}

// Floating particles in hero background
const heroBg = document.getElementById("hero-bg");
if (heroBg) {
  const particleCount = 18;
  for (let i = 0; i < particleCount; i++) {
    const p = document.createElement("div");
    p.className = "hero-particle";
    p.style.left = Math.random() * 100 + "%";
    p.style.top = Math.random() * 100 + "%";
    p.style.width = (4 + Math.random() * 6) + "px";
    p.style.height = p.style.width;
    heroBg.appendChild(p);
  }

  gsap.utils.toArray(".hero-particle").forEach((p) => {
    gsap.to(p, {
      y: "random(-60, 60)",
      x: "random(-40, 40)",
      opacity: "random(0.2, 0.6)",
      duration: "random(3, 6)",
      repeat: -1,
      yoyo: true,
      ease: "sine.inOut"
    });
  });
}

gsap.from(".hero p", {
  y: 30,
  opacity: 0,
  duration: 0.8,
  delay: 0.9,
  ease: "power2.out"
});

gsap.from(".hero .cta .btn", {
  scale: 0.9,
  opacity: 0,
  duration: 0.6,
  stagger: 0.15,
  delay: 1.1,
  ease: "back.out(1.7)"
});

// Fade-up elements
gsap.utils.toArray(".gsap-fade-up").forEach((el, i) => {
  gsap.from(el, {
    scrollTrigger: {
      trigger: el,
      start: "top 85%",
      toggleActions: "play none none reverse"
    },
    y: 40,
    opacity: 0,
    duration: 0.7,
    delay: i * 0.05,
    ease: "power2.out"
  });
});

// Fade-left elements (TOC, etc.)
gsap.utils.toArray(".gsap-fade-left").forEach((el) => {
  gsap.from(el, {
    scrollTrigger: {
      trigger: el,
      start: "top 85%",
      toggleActions: "play none none reverse"
    },
    x: -40,
    opacity: 0,
    duration: 0.7,
    ease: "power2.out"
  });
});

// Section reveals on scroll
gsap.utils.toArray(".gsap-reveal").forEach((el, i) => {
  gsap.from(el, {
    scrollTrigger: {
      trigger: el,
      start: "top 80%",
      toggleActions: "play none none reverse"
    },
    y: 50,
    opacity: 0,
    duration: 0.8,
    delay: (i % 3) * 0.1,
    ease: "power2.out"
  });
});

// Feature cards staggered animation
gsap.from(".feature", {
  scrollTrigger: {
    trigger: ".feature-grid",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 60,
  opacity: 0,
  duration: 0.7,
  stagger: 0.12,
  ease: "power2.out"
});

// Code blocks reveal
gsap.utils.toArray("pre").forEach((el) => {
  gsap.from(el, {
    scrollTrigger: {
      trigger: el,
      start: "top 85%",
      toggleActions: "play none none reverse"
    },
    y: 30,
    opacity: 0,
    scale: 0.98,
    duration: 0.6,
    ease: "power2.out"
  });
});

// Section headings slide in
gsap.utils.toArray("h2").forEach((el) => {
  gsap.from(el, {
    scrollTrigger: {
      trigger: el,
      start: "top 85%",
      toggleActions: "play none none reverse"
    },
    x: -30,
    opacity: 0,
    duration: 0.6,
    ease: "power2.out"
  });
});

// Button hover effects
document.querySelectorAll(".btn").forEach((btn) => {
  btn.addEventListener("mouseenter", () => {
    gsap.to(btn, { scale: 1.05, duration: 0.25, ease: "power2.out" });
  });
  btn.addEventListener("mouseleave", () => {
    gsap.to(btn, { scale: 1, duration: 0.25, ease: "power2.out" });
  });
});

// Logo gentle pulse on load
gsap.from(".logo img", {
  rotation: -10,
  scale: 0.8,
  opacity: 0,
  duration: 0.8,
  delay: 0.1,
  ease: "back.out(1.7)"
});

// Hero parallax on scroll
gsap.to(".hero h1", {
  scrollTrigger: {
    trigger: ".hero",
    start: "top top",
    end: "bottom top",
    scrub: true
  },
  y: 80,
  opacity: 0.3,
  ease: "none"
});

// Magnetic effect on nav links
document.querySelectorAll("nav a").forEach((link) => {
  link.addEventListener("mousemove", (e) => {
    const rect = link.getBoundingClientRect();
    const x = e.clientX - rect.left - rect.width / 2;
    const y = e.clientY - rect.top - rect.height / 2;
    gsap.to(link, { x: x * 0.25, y: y * 0.25, duration: 0.3, ease: "power2.out" });
  });
  link.addEventListener("mouseleave", () => {
    gsap.to(link, { x: 0, y: 0, duration: 0.3, ease: "power2.out" });
  });
});

// TOC link smooth scroll is handled by CSS scroll-behavior
// Highlight active TOC section on scroll
gsap.utils.toArray("section[id]").forEach((section) => {
  ScrollTrigger.create({
    trigger: section,
    start: "top center",
    end: "bottom center",
    onToggle: (self) => {
      const id = section.getAttribute("id");
      const link = document.querySelector(`.toc a[href="#${id}"]`);
      if (link) {
        gsap.to(link, { color: self.isActive ? "var(--accent)" : "var(--muted)", duration: 0.2 });
      }
    }
  });
});
