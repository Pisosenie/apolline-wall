for (const button of document.querySelectorAll("[data-copy]")) {
  button.addEventListener("click", async () => {
    const label = button.querySelector("span");
    try {
      await navigator.clipboard.writeText(button.dataset.copy);
      button.classList.add("done");
      if (label) label.textContent = "Copied";
    } catch {
      if (label) label.textContent = "Select and copy";
    }
    setTimeout(() => {
      button.classList.remove("done");
      if (label) label.textContent = "Copy";
    }, 1800);
  });
}

if ("IntersectionObserver" in window && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
  const shown = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (entry.isIntersecting) {
        entry.target.classList.add("seen");
        shown.unobserve(entry.target);
      }
    }
  }, { rootMargin: "0px 0px -8% 0px" });
  for (const el of document.querySelectorAll(".home .block .heading, .home .shots figure, .home .kind, .home .tier, .home .split > *, .home .desk, .home .install-card, .home .release")) {
    el.classList.add("reveal");
    shown.observe(el);
  }
}

const links = [...document.querySelectorAll(".toc a")];
if (links.length) {
  const byId = new Map(links.map((a) => [decodeURIComponent(a.hash.slice(1)), a]));
  const watch = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const link = byId.get(entry.target.id);
      if (!link) continue;
      for (const a of links) a.classList.toggle("on", a === link);
    }
  }, { rootMargin: "-80px 0px -70% 0px" });
  for (const id of byId.keys()) {
    const heading = document.getElementById(id);
    if (heading) watch.observe(heading);
  }
  if (matchMedia("(max-width: 960px)").matches) document.querySelector(".toc details")?.removeAttribute("open");
}

// The hero and the three monitors play the repository's own Shadertoy shader, the way the daemon
// does: only while someone can see them.
const accentOf = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
let accent = accentOf("#ff6a3d");
const players = [];

function compile(gl, type, text) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, text);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
  return shader;
}

function player(canvas, source) {
  const gl = canvas.getContext("webgl", { antialias: false, alpha: false, powerPreference: "low-power" });
  if (!gl) return null;
  const program = gl.createProgram();
  gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, "attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}"));
  gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER,
    "precision highp float;uniform vec3 iResolution;uniform float iTime;uniform vec3 iAccent;\n" + source +
    "\nvoid main(){mainImage(gl_FragColor,gl_FragCoord.xy);}"));
  gl.linkProgram(program);
  gl.useProgram(program);
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  const at = gl.getAttribLocation(program, "p");
  gl.enableVertexAttribArray(at);
  gl.vertexAttribPointer(at, 2, gl.FLOAT, false, 0, 0);
  const u = (name) => gl.getUniformLocation(program, name);
  const resolution = u("iResolution"), time = u("iTime"), tint = u("iAccent");
  const self = { canvas, seen: false, started: performance.now() };
  self.draw = (now) => {
    const scale = Math.min(window.devicePixelRatio || 1, 1.5) * 0.75;
    const w = Math.round(canvas.clientWidth * scale), h = Math.round(canvas.clientHeight * scale);
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    gl.viewport(0, 0, w, h);
    gl.uniform3f(resolution, w, h, 1);
    gl.uniform1f(time, (now - self.started) / 1000 + 40);
    gl.uniform3f(tint, accent[0], accent[1], accent[2]);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };
  return self;
}

const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
const status = document.querySelector("[data-state]");
let last = 0;
function frame(now) {
  requestAnimationFrame(frame);
  if (now - last < 33) return;
  last = now;
  for (const p of players) if (p.seen && !document.hidden) p.draw(now);
}

async function start() {
  const canvases = [...document.querySelectorAll("canvas[data-shader]")];
  if (!canvases.length) return;
  let source;
  try {
    source = await (await fetch(canvases[0].dataset.shader)).text();
  } catch {
    return;
  }
  for (const canvas of canvases) {
    try {
      const p = player(canvas, source);
      if (p) players.push(p);
    } catch {
      canvas.classList.add("off");
    }
  }
  const watch = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      const p = players.find((q) => q.canvas === entry.target);
      if (p) p.seen = entry.isIntersecting;
      if (p && p === players[0] && status) status.textContent = entry.isIntersecting ? "Playing" : "Paused";
    }
  });
  for (const p of players) {
    watch.observe(p.canvas);
    p.draw(performance.now());
  }
  if (!still) requestAnimationFrame(frame);
}
start();

for (const button of document.querySelectorAll("[data-accent]")) {
  button.addEventListener("click", () => {
    accent = accentOf(button.dataset.accent);
    document.documentElement.style.setProperty("--live", button.dataset.accent);
    for (const b of document.querySelectorAll("[data-accent]")) b.setAttribute("aria-pressed", String(b === button));
    if (still) for (const p of players) p.draw(performance.now());
  });
}
document.addEventListener("visibilitychange", () => {
  if (status && players[0]?.seen) status.textContent = document.hidden ? "Paused" : "Playing";
});
