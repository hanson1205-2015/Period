// Register GSAP plugins
gsap.registerPlugin(ScrollTrigger, TextPlugin);

// Header entrance
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

// Hero title: split into characters and animate
const heroTitle = document.getElementById("hero-title");
if (heroTitle) {
  const text = heroTitle.textContent;
  heroTitle.innerHTML = text
    .split("")
    .map((char) => `<span class="char">${char === " " ? "&nbsp;" : char}</span>`)
    .join("");

  gsap.from("#hero-title .char", {
    y: 100,
    opacity: 0,
    rotationX: -90,
    duration: 1,
    stagger: 0.08,
    delay: 0.2,
    ease: "back.out(1.7)"
  });
}

// Floating particles in hero background
const heroBg = document.getElementById("hero-bg");
if (heroBg) {
  const particleCount = 20;
  for (let i = 0; i < particleCount; i++) {
    const p = document.createElement("div");
    p.className = "hero-particle";
    p.style.left = Math.random() * 100 + "%";
    p.style.top = Math.random() * 100 + "%";
    const size = 4 + Math.random() * 6;
    p.style.width = size + "px";
    p.style.height = size + "px";
    heroBg.appendChild(p);
  }

  gsap.utils.toArray(".hero-particle").forEach((p) => {
    gsap.to(p, {
      y: "random(-70, 70)",
      x: "random(-50, 50)",
      opacity: "random(0.15, 0.5)",
      duration: "random(3, 7)",
      repeat: -1,
      yoyo: true,
      ease: "sine.inOut"
    });
  });
}

// Hero subtitle and CTA
gsap.from("#hero-subtitle", {
  y: 30,
  opacity: 0,
  duration: 0.8,
  delay: 1,
  ease: "power2.out"
});

gsap.from("#hero-cta .btn", {
  scale: 0.8,
  opacity: 0,
  duration: 0.7,
  stagger: 0.15,
  delay: 1.3,
  ease: "back.out(1.7)"
});

// Scroll indicator bounce
gsap.to(".scroll-indicator span", {
  y: 12,
  opacity: 0.3,
  duration: 1,
  repeat: -1,
  yoyo: true,
  ease: "power1.inOut"
});

// Fade out scroll indicator on scroll
ScrollTrigger.create({
  trigger: ".hero",
  start: "top top",
  end: "50% top",
  scrub: true,
  onUpdate: (self) => {
    gsap.set(".scroll-indicator", { opacity: 1 - self.progress });
  }
});

// Hero parallax on scroll
gsap.to(".hero h1", {
  scrollTrigger: {
    trigger: ".hero",
    start: "top top",
    end: "bottom top",
    scrub: true
  },
  y: 100,
  opacity: 0.2,
  ease: "none"
});

gsap.to("#hero-subtitle, #hero-cta", {
  scrollTrigger: {
    trigger: ".hero",
    start: "top top",
    end: "bottom top",
    scrub: true
  },
  y: 60,
  opacity: 0,
  ease: "none"
});

// Why Period section
gsap.from("#why-period h2", {
  scrollTrigger: {
    trigger: "#why-period",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  x: -40,
  opacity: 0,
  duration: 0.7,
  ease: "power2.out"
});

gsap.from(".index-feature", {
  scrollTrigger: {
    trigger: ".feature-grid",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 80,
  opacity: 0,
  rotateX: 10,
  duration: 0.8,
  stagger: 0.12,
  ease: "power2.out"
});

// Feature card hover tilt
if (!window.matchMedia("(pointer: coarse)").matches) {
  document.querySelectorAll(".index-feature").forEach((card) => {
    card.addEventListener("mousemove", (e) => {
      const rect = card.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const centerX = rect.width / 2;
      const centerY = rect.height / 2;
      const rotateX = (y - centerY) / 20;
      const rotateY = (centerX - x) / 20;
      gsap.to(card, {
        rotationX: rotateX,
        rotationY: rotateY,
        transformPerspective: 800,
        scale: 1.03,
        duration: 0.3,
        ease: "power2.out"
      });
    });
    card.addEventListener("mouseleave", () => {
      gsap.to(card, {
        rotationX: 0,
        rotationY: 0,
        scale: 1,
        duration: 0.4,
        ease: "power2.out"
      });
    });
  });
}

// Hello Period section with typewriter effect
const typewriter = document.getElementById("typewriter");
if (typewriter) {
  const originalHTML = typewriter.innerHTML;
  typewriter.innerHTML = "";
  typewriter.style.opacity = 1;

  const wrapper = document.createElement("code");
  typewriter.appendChild(wrapper);

  gsap.from("#hello-period h2", {
    scrollTrigger: {
      trigger: "#hello-period",
      start: "top 80%",
      toggleActions: "play none none reverse"
    },
    x: -40,
    opacity: 0,
    duration: 0.7,
    ease: "power2.out"
  });

  ScrollTrigger.create({
    trigger: "#hello-period",
    start: "top 75%",
    once: true,
    onEnter: () => {
      typewriter.innerHTML = originalHTML;
      const code = typewriter.querySelector("code");
      code.innerHTML = "";

      const tokens = [
        { text: "-- Greet the world.\n", class: "comment" },
        { text: "let ", class: "keyword" },
        { text: "greeting ", class: "" },
        { text: "be ", class: "keyword" },
        { text: '"Hello, World!"', class: "string" },
        { text: ".\n", class: "period" },
        { text: "show ", class: "builtin" },
        { text: "greeting", class: "" },
        { text: ".", class: "period" }
      ];

      let tl = gsap.timeline();
      tokens.forEach((token) => {
        const span = document.createElement("span");
        if (token.class) span.className = token.class;
        code.appendChild(span);
        tl.to(span, {
          text: { value: token.text, delimiter: "" },
          duration: token.text.length * 0.03,
          ease: "none"
        }, "+=0.02");
      });

      tl.to(typewriter, {
        boxShadow: "0 0 30px rgba(110, 168, 255, 0.15)",
        duration: 0.6,
        yoyo: true,
        repeat: 1,
        ease: "power2.inOut"
      });
    }
  });
}

// Button hover effects
document.querySelectorAll(".btn").forEach((btn) => {
  btn.addEventListener("mouseenter", () => {
    gsap.to(btn, { scale: 1.06, duration: 0.25, ease: "power2.out" });
  });
  btn.addEventListener("mouseleave", () => {
    gsap.to(btn, { scale: 1, duration: 0.25, ease: "power2.out" });
  });
});

// Logo entrance
gsap.from(".logo img", {
  rotation: -15,
  scale: 0.7,
  opacity: 0,
  duration: 0.9,
  delay: 0.1,
  ease: "back.out(1.7)"
});

// Magnetic nav links
document.querySelectorAll("nav a").forEach((link) => {
  link.addEventListener("mousemove", (e) => {
    const rect = link.getBoundingClientRect();
    const x = e.clientX - rect.left - rect.width / 2;
    const y = e.clientY - rect.top - rect.height / 2;
    gsap.to(link, { x: x * 0.3, y: y * 0.3, duration: 0.3, ease: "power2.out" });
  });
  link.addEventListener("mouseleave", () => {
    gsap.to(link, { x: 0, y: 0, duration: 0.3, ease: "power2.out" });
  });
});
