<script setup lang="ts">
import { withBase } from 'vitepress'
import DamageDemo from './DamageDemo.vue'

const stats = [
  {
    value: '24 µs',
    label: 'one small box changes among 600 in a 1920 by 1080 frame',
    note: 'against 3.7 ms to redraw everything',
  },
  {
    value: '37 µs',
    label: 'one keystroke into a focused text field',
    note: 'against 87 µs for a full redraw',
  },
  {
    value: '0.31 ms',
    label: 'one scroll step through a thousand mixed rows',
    note: 'the rows keep their paint and only move',
  },
]

const features = [
  {
    title: 'Repaints only what changed',
    body: 'Every frame works out which pixels a change touched and redraws exactly those. A frame that changes one small box rasterizes and presents one small box.',
    icon: 'damage',
  },
  {
    title: 'Fine-grained signals',
    body: 'State lives in signals. A closure that reads a signal binds that one property of that one widget, so a write updates what depends on it and nothing else.',
    icon: 'signal',
  },
  {
    title: 'Fast without a GPU',
    body: 'A CPU renderer that is fast on its own, for virtual machines, remote desktops and older laptops. A GPU renderer takes over when a real adapter is there.',
    icon: 'cpu',
  },
  {
    title: 'A retained widget tree',
    body: 'Widgets are built once and kept. A change that only affects color skips layout, and each node keeps its paint until something it shows changes.',
    icon: 'tree',
  },
  {
    title: 'Text and input',
    body: 'Shaped text, a text input with selection, undo and redo, input methods for composing languages, and the system clipboard.',
    icon: 'text',
  },
  {
    title: 'Desktop and the web',
    body: 'One window API for the desktop, tested on Linux in this release, and the CPU renderer in a browser canvas. Desktop builds expose the widget tree to screen readers.',
    icon: 'web',
  },
]
</script>

<template>
  <div class="home">
    <section class="hero">
      <div class="copy">
        <p class="eyebrow"><span class="dot" />Rust GUI library <span class="ver">v0.1</span></p>
        <h1 class="title">
          Repaint only<br />
          <span class="mark">what changed.</span>
        </h1>
        <p class="lead">
          Smudge is a native Rust GUI library with a retained widget tree,
          fine-grained reactive signals and damage-tracked repainting. It is
          built to stay fast where there is no usable GPU, and it runs on the
          desktop and in the browser.
        </p>
        <div class="actions">
          <a class="btn primary" :href="withBase('/guide/quick-start')">Get started</a>
          <a class="btn" :href="withBase('/guide/introduction')">Why Smudge</a>
          <a class="btn ghost" href="https://docs.rs/smudge" target="_blank" rel="noreferrer">
            API on docs.rs
          </a>
        </div>
        <div class="install">
          <span class="prompt">$</span>
          <code>cargo add smudge</code>
        </div>
      </div>
      <div class="visual">
        <DamageDemo />
      </div>
    </section>

    <section class="stats" aria-label="Measured frame times">
      <div v-for="stat in stats" :key="stat.value" class="stat">
        <div class="value">{{ stat.value }}</div>
        <div class="label">{{ stat.label }}</div>
        <div class="note">{{ stat.note }}</div>
      </div>
      <p class="method">
        Median frame on the CPU renderer, one thread of a Ryzen 7 7700, headless.
        <a :href="withBase('/internals/performance')">How these are measured</a>
      </p>
    </section>

    <section class="features">
      <article v-for="feature in features" :key="feature.title" class="feature">
        <div class="icon" :class="feature.icon" aria-hidden="true">
          <svg v-if="feature.icon === 'damage'" viewBox="0 0 32 32">
            <rect x="3" y="3" width="26" height="26" rx="4" class="soft" />
            <rect x="9" y="9" width="14" height="10" rx="2" class="dash" />
            <rect x="12" y="12" width="8" height="4" rx="1" class="solid" />
          </svg>
          <svg v-else-if="feature.icon === 'signal'" viewBox="0 0 32 32">
            <circle cx="8" cy="16" r="4" class="solid" />
            <path d="M12 16 H 19" class="line" />
            <circle cx="24" cy="9" r="3" class="soft-dot" />
            <circle cx="24" cy="23" r="3" class="ring" />
            <path d="M19 16 L 21.5 10.5 M 19 16 L 21.5 21.5" class="line" />
          </svg>
          <svg v-else-if="feature.icon === 'cpu'" viewBox="0 0 32 32">
            <rect x="8" y="8" width="16" height="16" rx="3" class="ring" />
            <rect x="12" y="12" width="8" height="8" rx="1.5" class="solid" />
            <path d="M12 4 V 8 M 16 4 V 8 M 20 4 V 8 M 12 24 V 28 M 16 24 V 28 M 20 24 V 28 M 4 12 H 8 M 4 16 H 8 M 4 20 H 8 M 24 12 H 28 M 24 16 H 28 M 24 20 H 28" class="line" />
          </svg>
          <svg v-else-if="feature.icon === 'tree'" viewBox="0 0 32 32">
            <rect x="12" y="3" width="8" height="6" rx="1.5" class="solid" />
            <path d="M16 9 V 14 M 8 14 H 24 M 8 14 V 18 M 24 14 V 18 M 16 14 V 18" class="line" />
            <rect x="4" y="18" width="8" height="6" rx="1.5" class="ring" />
            <rect x="12" y="18" width="8" height="6" rx="1.5" class="ring" />
            <rect x="20" y="18" width="8" height="6" rx="1.5" class="dash" />
          </svg>
          <svg v-else-if="feature.icon === 'text'" viewBox="0 0 32 32">
            <rect x="3" y="8" width="26" height="16" rx="4" class="ring" />
            <path d="M8 13 H 16 M 12 13 V 20" class="line" />
            <path d="M21 11.5 V 20.5" class="caret" />
          </svg>
          <svg v-else viewBox="0 0 32 32">
            <rect x="3" y="6" width="17" height="13" rx="2.5" class="ring" />
            <path d="M8 23 H 15 M 11.5 19 V 23" class="line" />
            <rect x="17" y="12" width="12" height="15" rx="2.5" class="solid" />
          </svg>
        </div>
        <h3>{{ feature.title }}</h3>
        <p>{{ feature.body }}</p>
      </article>
    </section>
  </div>
</template>

<style scoped>
.home {
  max-width: 1152px;
  margin: 0 auto;
  padding: 0 24px;
}

.hero {
  display: grid;
  grid-template-columns: 1.05fr 1fr;
  gap: 56px;
  align-items: center;
  padding: 72px 0 56px;
}

.eyebrow {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin: 0 0 22px;
  padding: 5px 12px 5px 10px;
  border: 1px dashed var(--vp-c-border);
  border-radius: 999px;
  font-size: 13px;
  font-weight: 600;
  color: var(--vp-c-text-2);
}

.eyebrow .dot {
  width: 8px;
  height: 8px;
  border-radius: 2px;
  background: var(--sm-lime-deep);
  box-shadow: 0 0 0 3px var(--vp-c-brand-soft);
}

.eyebrow .ver {
  font-family: var(--vp-font-family-mono);
  font-size: 12px;
  color: var(--vp-c-text-3);
}

.title {
  margin: 0;
  font-family: var(--sm-font-display);
  font-size: clamp(44px, 6.4vw, 76px);
  font-weight: 700;
  line-height: 1.02;
  letter-spacing: -0.045em;
  color: var(--vp-home-hero-name-color);
}

.mark {
  position: relative;
  white-space: nowrap;
  z-index: 0;
}

.mark::before {
  content: '';
  position: absolute;
  left: -0.08em;
  right: -0.1em;
  top: 0.52em;
  bottom: 0.02em;
  z-index: -1;
  border-radius: 0.12em;
  background: var(--sm-lime);
  transform: skewX(-8deg) rotate(-1deg);
  opacity: 0.85;
}

.dark .mark {
  color: #f4f5f8;
}

.dark .mark::before {
  opacity: 0.32;
}

.lead {
  max-width: 540px;
  margin: 26px 0 0;
  font-size: 18px;
  line-height: 1.65;
  color: var(--vp-c-text-2);
}

.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  margin-top: 32px;
}

.btn {
  display: inline-flex;
  align-items: center;
  height: 42px;
  padding: 0 18px;
  border-radius: 10px;
  border: 1px solid var(--vp-c-border);
  font-family: var(--sm-font-display);
  font-size: 15px;
  font-weight: 650;
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg-elv);
  text-decoration: none;
  transition: border-color 0.15s, background-color 0.15s, transform 0.15s;
}

.btn:hover {
  border-color: var(--vp-c-brand-3);
  transform: translateY(-1px);
}

.btn.primary {
  border-color: transparent;
  color: #101115;
  background: var(--sm-lime);
}

.btn.primary:hover {
  background: var(--sm-lime-bright);
}

.btn.ghost {
  background: transparent;
  border-style: dashed;
}

.install {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  margin-top: 22px;
  padding: 9px 14px;
  border-radius: 9px;
  background: var(--vp-code-block-bg);
  font-family: var(--vp-font-family-mono);
  font-size: 13.5px;
}

.install .prompt {
  color: #5d6370;
}

.install code {
  color: #e6e8ee;
}

.visual {
  display: flex;
  justify-content: center;
  position: relative;
}

.visual::before {
  content: '';
  position: absolute;
  inset: -40px -20px;
  z-index: -1;
  background-image: radial-gradient(var(--vp-c-divider) 1.2px, transparent 1.2px);
  background-size: 16px 16px;
  mask-image: radial-gradient(closest-side, #000 40%, transparent);
}

.stats {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  padding: 8px 0 24px;
}

.stat {
  padding: 22px 22px 20px;
  border: 1px dashed var(--vp-c-border);
  border-radius: 14px;
  background: var(--vp-c-bg-soft);
}

.stat .value {
  font-family: var(--sm-font-display);
  font-size: 38px;
  font-weight: 700;
  letter-spacing: -0.03em;
  color: var(--vp-c-text-1);
}

.stat .label {
  margin-top: 6px;
  font-size: 15px;
  line-height: 1.5;
  color: var(--vp-c-text-1);
}

.stat .note {
  margin-top: 8px;
  font-size: 13px;
  color: var(--vp-c-text-3);
}

.method {
  grid-column: 1 / -1;
  margin: 2px 0 0;
  font-size: 13px;
  color: var(--vp-c-text-3);
}

.method a {
  margin-left: 6px;
  color: var(--vp-c-brand-1);
  font-weight: 600;
  text-underline-offset: 3px;
}

.features {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  padding: 40px 0 24px;
}

.feature {
  padding: 24px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 14px;
  background: var(--vp-c-bg);
  transition: border-color 0.2s, transform 0.2s;
}

.feature:hover {
  border-color: var(--vp-c-brand-3);
  transform: translateY(-2px);
}

.feature h3 {
  margin: 16px 0 8px;
  font-family: var(--sm-font-display);
  font-size: 18px;
  font-weight: 650;
  letter-spacing: -0.015em;
}

.feature p {
  margin: 0;
  font-size: 15px;
  line-height: 1.6;
  color: var(--vp-c-text-2);
}

.icon {
  width: 44px;
  height: 44px;
  padding: 6px;
  border-radius: 11px;
  background: var(--sm-graphite-1);
}

.icon svg {
  width: 100%;
  height: 100%;
  fill: none;
}

.icon .solid {
  fill: var(--sm-lime);
}

.icon .soft {
  fill: rgba(255, 255, 255, 0.05);
  stroke: #3a3f4b;
  stroke-width: 1.5;
}

.icon .soft-dot {
  fill: #3a3f4b;
}

.icon .ring {
  stroke: #8a909c;
  stroke-width: 1.8;
}

.icon .dash {
  stroke: var(--sm-lime);
  stroke-width: 1.6;
  stroke-dasharray: 3 2.4;
}

.icon .line {
  stroke: #8a909c;
  stroke-width: 1.8;
  stroke-linecap: round;
}

.icon .caret {
  stroke: var(--sm-lime);
  stroke-width: 2.2;
  stroke-linecap: round;
}

@media (max-width: 960px) {
  .hero {
    grid-template-columns: 1fr;
    gap: 44px;
    padding-top: 48px;
  }

  .stats,
  .features {
    grid-template-columns: 1fr 1fr;
  }
}

@media (max-width: 640px) {
  .stats,
  .features {
    grid-template-columns: 1fr;
  }
}
</style>
