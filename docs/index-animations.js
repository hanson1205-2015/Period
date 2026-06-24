// Register GSAP plugins
gsap.registerPlugin(ScrollTrigger, TextPlugin);

// ===== Header entrance =====
gsap.from("header", {
  y: -40,
  opacity: 0,
  duration: 0.8,
  ease: "power3.out"
});

gsap.from("nav a", {
  y: -20,
  opacity: 0,
  duration: 0.5,
  stagger: 0.08,
  delay: 0.4,
  ease: "power2.out"
});

gsap.from(".logo img", {
  rotation: -20,
  scale: 0.6,
  opacity: 0,
  duration: 0.9,
  delay: 0.1,
  ease: "back.out(1.7)"
});

// ===== Hero title character animation =====
const heroTitle = document.getElementById("hero-title");
if (heroTitle) {
  const text = heroTitle.innerHTML;
  heroTitle.innerHTML = text
    .split(/(<[^>]+>)/)
    .map((part) => {
      if (part.startsWith("<")) return part;
      return part
        .split("")
        .map((char) => `<span class="char">${char === " " ? "&nbsp;" : char}</span>`)
        .join("");
    })
    .join("");

  gsap.from("#hero-title .char", {
    y: 120,
    opacity: 0,
    rotationX: -90,
    scale: 0.5,
    duration: 1.2,
    stagger: 0.08,
    delay: 0.3,
    ease: "back.out(1.2)"
  });

  // Period dot pulse
  gsap.to("#hero-title .period", {
    scale: 1.2,
    textShadow: "0 0 40px rgba(110, 168, 255, 0.8)",
    duration: 1,
    repeat: -1,
    yoyo: true,
    ease: "power1.inOut"
  });
}

// ===== Hero badge =====
gsap.from("#hero-badge", {
  y: 20,
  opacity: 0,
  duration: 0.6,
  delay: 0.2,
  ease: "power2.out"
});

// ===== Floating particles =====
const heroBg = document.getElementById("hero-bg");
if (heroBg) {
  const particleCount = 30;
  for (let i = 0; i < particleCount; i++) {
    const p = document.createElement("div");
    p.className = "hero-particle";
    p.style.left = Math.random() * 100 + "%";
    p.style.top = Math.random() * 100 + "%";
    const size = 3 + Math.random() * 7;
    p.style.width = size + "px";
    p.style.height = size + "px";
    p.style.opacity = 0.2 + Math.random() * 0.5;
    heroBg.appendChild(p);
  }

  gsap.utils.toArray(".hero-particle").forEach((p) => {
    gsap.to(p, {
      y: "random(-100, 100)",
      x: "random(-60, 60)",
      opacity: "random(0.15, 0.6)",
      scale: "random(0.6, 1.4)",
      duration: "random(4, 9)",
      repeat: -1,
      yoyo: true,
      ease: "sine.inOut"
    });
  });
}

// ===== Animated gradient blobs =====
const blob1 = document.getElementById("blob-1");
const blob2 = document.getElementById("blob-2");
if (blob1 && blob2) {
  blob1.style.cssText = "width: 500px; height: 500px; background: rgba(110, 168, 255, 0.2); top: -100px; left: -100px;";
  blob2.style.cssText = "width: 600px; height: 600px; background: rgba(188, 146, 255, 0.15); bottom: -150px; right: -150px;";

  gsap.to(blob1, {
    x: 200,
    y: 150,
    scale: 1.2,
    duration: 12,
    repeat: -1,
    yoyo: true,
    ease: "sine.inOut"
  });

  gsap.to(blob2, {
    x: -180,
    y: -120,
    scale: 1.1,
    duration: 15,
    repeat: -1,
    yoyo: true,
    ease: "sine.inOut"
  });
}

// ===== Hero subtitle & CTA =====
gsap.from("#hero-subtitle", {
  y: 40,
  opacity: 0,
  duration: 0.9,
  delay: 1,
  ease: "power2.out"
});

gsap.from("#hero-cta .btn", {
  y: 30,
  opacity: 0,
  duration: 0.7,
  stagger: 0.15,
  delay: 1.3,
  ease: "back.out(1.7)"
});

// ===== Scroll indicator =====
gsap.to(".scroll-indicator span", {
  y: 14,
  opacity: 0.3,
  duration: 1.2,
  repeat: -1,
  yoyo: true,
  ease: "power1.inOut"
});

ScrollTrigger.create({
  trigger: ".hero",
  start: "top top",
  end: "50% top",
  scrub: true,
  onUpdate: (self) => {
    gsap.set(".scroll-indicator", { opacity: 1 - self.progress });
  }
});

// ===== Hero parallax on scroll =====
gsap.to("#hero-title", {
  scrollTrigger: {
    trigger: ".hero",
    start: "top top",
    end: "bottom top",
    scrub: true
  },
  y: 120,
  opacity: 0.1,
  filter: "blur(10px)",
  ease: "none"
});

gsap.to("#hero-subtitle, #hero-cta, #hero-badge", {
  scrollTrigger: {
    trigger: ".hero",
    start: "top top",
    end: "bottom top",
    scrub: true
  },
  y: 80,
  opacity: 0,
  ease: "none"
});

// ===== Stats count-up animation =====
gsap.utils.toArray(".stat-number").forEach((stat) => {
  const target = parseInt(stat.dataset.value, 10);
  ScrollTrigger.create({
    trigger: stat,
    start: "top 85%",
    once: true,
    onEnter: () => {
      gsap.to(stat, {
        innerText: target,
        duration: 2,
        snap: { innerText: 1 },
        ease: "power2.out"
      });
    }
  });
});

gsap.from(".stat", {
  scrollTrigger: {
    trigger: ".stats-grid",
    start: "top 85%",
    toggleActions: "play none none reverse"
  },
  y: 60,
  opacity: 0,
  duration: 0.7,
  stagger: 0.1,
  ease: "power2.out"
});

// ===== Features section =====
gsap.from("#features .section-label, #features h2, #features > p", {
  scrollTrigger: {
    trigger: "#features",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 40,
  opacity: 0,
  duration: 0.8,
  stagger: 0.15,
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
  rotationX: 15,
  duration: 0.8,
  stagger: 0.1,
  ease: "power2.out"
});

// Feature card hover tilt (desktop only)
if (!window.matchMedia("(pointer: coarse)").matches) {
  document.querySelectorAll(".index-feature").forEach((card) => {
    card.addEventListener("mousemove", (e) => {
      const rect = card.getBoundingClientRect();
      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;
      const centerX = rect.width / 2;
      const centerY = rect.height / 2;
      const rotateX = (y - centerY) / 15;
      const rotateY = (centerX - x) / 15;
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

// ===== How it works section =====
gsap.from("#how-it-works .section-label, #how-it-works h2, #how-it-works > p", {
  scrollTrigger: {
    trigger: "#how-it-works",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 40,
  opacity: 0,
  duration: 0.8,
  stagger: 0.15,
  ease: "power2.out"
});

gsap.from(".index-step", {
  scrollTrigger: {
    trigger: ".steps",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 60,
  opacity: 0,
  duration: 0.8,
  stagger: 0.2,
  ease: "power2.out"
});

// Step numbers spin on scroll
gsap.utils.toArray(".step-number").forEach((num) => {
  gsap.from(num, {
    scrollTrigger: {
      trigger: num,
      start: "top 85%",
      toggleActions: "play none none reverse"
    },
    rotation: 360,
    scale: 0,
    duration: 0.8,
    ease: "back.out(1.7)"
  });
});

// ===== Code showcase with typewriter =====
const typewriter = document.getElementById("typewriter");
const outputBody = document.getElementById("code-output-body");

if (typewriter && outputBody) {
  const originalHTML = typewriter.innerHTML;
  typewriter.innerHTML = "";
  const wrapper = document.createElement("code");
  typewriter.appendChild(wrapper);

  gsap.from("#hello-period .section-label, #hello-period h2, #hello-period > p", {
    scrollTrigger: {
      trigger: "#hello-period",
      start: "top 80%",
      toggleActions: "play none none reverse"
    },
    y: 40,
    opacity: 0,
    duration: 0.8,
    stagger: 0.15,
    ease: "power2.out"
  });

  gsap.from("#code-output", {
    scrollTrigger: {
      trigger: ".code-showcase",
      start: "top 80%",
      toggleActions: "play none none reverse"
    },
    x: 50,
    opacity: 0,
    duration: 0.8,
    delay: 0.4,
    ease: "power2.out"
  });

  ScrollTrigger.create({
    trigger: "#hello-period",
    start: "top 70%",
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
          duration: token.text.length * 0.04,
          ease: "none"
        }, "+=0.03");
      });

      tl.add(() => {
        outputBody.innerHTML = "";
        const lines = [
          "> Hello, World!",
          "> Program finished."
        ];
        lines.forEach((line, i) => {
          setTimeout(() => {
            const div = document.createElement("div");
            div.textContent = line;
            outputBody.appendChild(div);
            gsap.from(div, { opacity: 0, x: -10, duration: 0.3 });
          }, i * 600);
        });
      }, "+=0.3");

      tl.to(typewriter, {
        boxShadow: "0 0 40px rgba(110, 168, 255, 0.15)",
        duration: 0.8,
        yoyo: true,
        repeat: 1,
        ease: "power2.inOut"
      }, "-=0.5");
    }
  });
}

// ===== Ecosystem section =====
gsap.from("#ecosystem .section-label, #ecosystem h2, #ecosystem > p", {
  scrollTrigger: {
    trigger: "#ecosystem",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 40,
  opacity: 0,
  duration: 0.8,
  stagger: 0.15,
  ease: "power2.out"
});

gsap.from("#ecosystem .index-feature", {
  scrollTrigger: {
    trigger: "#ecosystem .feature-grid",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 60,
  opacity: 0,
  duration: 0.8,
  stagger: 0.12,
  ease: "power2.out"
});

// ===== CTA section =====
gsap.from(".cta-section h2, .cta-section p, .cta-section .cta", {
  scrollTrigger: {
    trigger: ".cta-section",
    start: "top 80%",
    toggleActions: "play none none reverse"
  },
  y: 50,
  opacity: 0,
  duration: 0.8,
  stagger: 0.15,
  ease: "power2.out"
});

// ===== Button hover effects =====
document.querySelectorAll(".btn").forEach((btn) => {
  btn.addEventListener("mouseenter", () => {
    gsap.to(btn, { scale: 1.06, duration: 0.25, ease: "power2.out" });
  });
  btn.addEventListener("mouseleave", () => {
    gsap.to(btn, { scale: 1, duration: 0.25, ease: "power2.out" });
  });
});

// ===== Magnetic nav links =====
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

// ===== Mouse-following spotlight in hero =====
const hero = document.querySelector(".hero");
if (hero && !window.matchMedia("(pointer: coarse)").matches) {
  const spotlight = document.createElement("div");
  spotlight.style.cssText = `
    position: absolute;
    width: 400px;
    height: 400px;
    background: radial-gradient(circle, rgba(110, 168, 255, 0.08) 0%, transparent 70%);
    border-radius: 50%;
    pointer-events: none;
    z-index: -1;
    transform: translate(-50%, -50%);
  `;
  hero.appendChild(spotlight);

  hero.addEventListener("mousemove", (e) => {
    const rect = hero.getBoundingClientRect();
    gsap.to(spotlight, {
      x: e.clientX - rect.left,
      y: e.clientY - rect.top,
      duration: 0.5,
      ease: "power2.out"
    });
  });
}
