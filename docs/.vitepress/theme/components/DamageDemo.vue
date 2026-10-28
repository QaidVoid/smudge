<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'

interface Damage {
  x: number
  y: number
  w: number
  h: number
}

const win = ref<HTMLElement | null>(null)
const countEl = ref<HTMLElement | null>(null)
const buttonEl = ref<HTMLElement | null>(null)
const meterEl = ref<HTMLElement | null>(null)
const clockEl = ref<HTMLElement | null>(null)
const itemEls = ref<HTMLElement[]>([])

const items = ['Inbox', 'Drafts', 'Starred', 'Archive', 'Trash']
const count = ref(41)
const pressed = ref(false)
const selected = ref(0)
const progress = ref(38)
const seconds = ref(7)
const frame = ref(1284)
const damage = ref<Damage[]>([])
const flash = ref(0)
const area = ref({ w: 1, h: 1 })

const clock = computed(() => `12:04:${String(seconds.value % 60).padStart(2, '0')}`)

const repainted = computed(() =>
  damage.value.reduce((sum, rect) => sum + rect.w * rect.h, 0),
)
const share = computed(() => (100 * repainted.value) / (area.value.w * area.value.h))

function rectOf(el: HTMLElement | null | undefined): Damage | null {
  if (!el || !win.value) return null
  const outer = win.value.getBoundingClientRect()
  const inner = el.getBoundingClientRect()
  const pad = 3
  return {
    x: inner.left - outer.left - pad,
    y: inner.top - outer.top - pad,
    w: inner.width + 2 * pad,
    h: inner.height + 2 * pad,
  }
}

function show(...els: (HTMLElement | null | undefined)[]) {
  if (win.value) {
    const box = win.value.getBoundingClientRect()
    area.value = { w: box.width, h: box.height }
  }
  damage.value = els.map(rectOf).filter((rect): rect is Damage => rect !== null)
  frame.value += 1
  flash.value += 1
}

let step = 0
async function advance() {
  switch (step % 4) {
    case 0: {
      count.value += 1
      pressed.value = true
      await nextTick()
      show(countEl.value, buttonEl.value)
      setTimeout(() => (pressed.value = false), 260)
      break
    }
    case 1: {
      const from = selected.value
      selected.value = (selected.value + 1) % items.length
      await nextTick()
      show(itemEls.value[from], itemEls.value[selected.value])
      break
    }
    case 2: {
      progress.value = progress.value >= 90 ? 24 : progress.value + 13
      await nextTick()
      show(meterEl.value)
      break
    }
    default: {
      seconds.value += 1
      await nextTick()
      show(clockEl.value)
    }
  }
  step += 1
}

let timer: ReturnType<typeof setInterval> | undefined

onMounted(() => {
  const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches
  advance()
  if (!still) timer = setInterval(advance, 1500)
})

onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>

<template>
  <figure class="demo" aria-label="An application window where each frame repaints only the element that changed">
    <div ref="win" class="window">
      <div class="titlebar">
        <span class="lights"><i /><i /><i /></span>
        <span class="title">mail.rs</span>
      </div>
      <div class="body">
        <aside class="side">
          <div
            v-for="(item, index) in items"
            :key="item"
            ref="itemEls"
            class="item"
            :class="{ active: index === selected }"
          >
            <span class="glyph" />{{ item }}
          </div>
        </aside>
        <section class="main">
          <div class="card">
            <div class="row">
              <div>
                <div ref="countEl" class="count">{{ count }}</div>
                <div class="caption">messages read</div>
              </div>
              <button ref="buttonEl" class="button" :class="{ pressed }" tabindex="-1">
                Mark read
              </button>
            </div>
          </div>
          <div class="card">
            <div class="caption">Syncing</div>
            <div ref="meterEl" class="meter">
              <div class="fill" :style="{ width: progress + '%' }" />
            </div>
          </div>
          <div class="card lines">
            <i style="width: 82%" /><i style="width: 64%" /><i style="width: 72%" />
          </div>
          <div class="status">
            <span class="caption">Last sync</span>
            <span ref="clockEl" class="clock">{{ clock }}</span>
          </div>
        </section>
      </div>
      <div
        v-for="(rect, index) in damage"
        :key="`${flash}-${index}`"
        class="damage"
        :style="{
          left: rect.x + 'px',
          top: rect.y + 'px',
          width: rect.w + 'px',
          height: rect.h + 'px',
        }"
      />
    </div>
    <figcaption class="hud">
      <span>frame {{ frame.toLocaleString('en-US') }}</span>
      <span class="sep" />
      <span>
        repainted <b>{{ share.toFixed(1) }}%</b> of the window
      </span>
      <span class="sep" />
      <span>{{ damage.length }} {{ damage.length === 1 ? 'rect' : 'rects' }}</span>
    </figcaption>
  </figure>
</template>

<style scoped>
.demo {
  margin: 0;
  width: 100%;
  max-width: 520px;
}

.window {
  position: relative;
  border-radius: 14px;
  background: #16181d;
  border: 1px solid #2a2e37;
  box-shadow:
    0 30px 80px -30px rgba(0, 0, 0, 0.55),
    0 0 0 1px rgba(255, 255, 255, 0.02) inset;
  overflow: hidden;
  color: #d6d9e0;
  font-family: var(--vp-font-family-base);
  user-select: none;
}

.titlebar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 34px;
  padding: 0 12px;
  background: #101115;
  border-bottom: 1px solid #22252c;
}

.lights {
  display: flex;
  gap: 6px;
}

.lights i {
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #2f333c;
}

.title {
  font-family: var(--vp-font-family-mono);
  font-size: 11px;
  color: #7f8592;
}

.body {
  display: grid;
  grid-template-columns: 34% 1fr;
  min-height: 268px;
}

.side {
  padding: 10px 8px;
  background: #121418;
  border-right: 1px solid #22252c;
}

.item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  margin-bottom: 2px;
  border-radius: 7px;
  font-size: 12.5px;
  color: #9ba1ad;
}

.item.active {
  background: #262a32;
  color: #eceef3;
}

.glyph {
  width: 9px;
  height: 9px;
  border-radius: 3px;
  background: #3a3f4b;
}

.item.active .glyph {
  background: #d4ff3f;
}

.main {
  display: flex;
  flex-direction: column;
  gap: 9px;
  padding: 12px;
}

.card {
  padding: 10px 12px;
  border-radius: 9px;
  background: #1c1f25;
  border: 1px solid #252931;
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.count {
  display: inline-block;
  font-family: var(--sm-font-display);
  font-size: 26px;
  font-weight: 700;
  line-height: 1.1;
  color: #f4f5f8;
  font-variant-numeric: tabular-nums;
}

.caption {
  font-size: 11px;
  color: #7f8592;
}

.button {
  padding: 6px 11px;
  border-radius: 7px;
  font-size: 12px;
  font-weight: 600;
  color: #101115;
  background: #d4ff3f;
  cursor: default;
  transition: background-color 0.12s, transform 0.12s;
}

.button.pressed {
  background: #9fe000;
  transform: translateY(1px);
}

.meter {
  height: 7px;
  margin-top: 8px;
  border-radius: 4px;
  background: #2a2e37;
  overflow: hidden;
}

.fill {
  height: 100%;
  border-radius: 4px;
  background: linear-gradient(90deg, #9fe000, #d4ff3f);
}

.lines {
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.lines i {
  display: block;
  height: 6px;
  border-radius: 3px;
  background: #2a2e37;
}

.status {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 2px;
}

.clock {
  display: inline-block;
  font-family: var(--vp-font-family-mono);
  font-size: 11.5px;
  color: #b3b8c3;
  font-variant-numeric: tabular-nums;
}

.damage {
  position: absolute;
  pointer-events: none;
  border: 1.5px dashed rgba(212, 255, 63, 0.9);
  border-radius: 6px;
  background: rgba(212, 255, 63, 0.13);
  animation: damage 1.4s ease-out forwards;
}

@keyframes damage {
  0% {
    opacity: 0;
    transform: scale(1.04);
  }
  12% {
    opacity: 1;
    transform: scale(1);
  }
  70% {
    opacity: 1;
  }
  100% {
    opacity: 0.25;
  }
}

.hud {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  margin-top: 14px;
  font-family: var(--vp-font-family-mono);
  font-size: 12px;
  color: var(--vp-c-text-3);
}

.hud b {
  color: var(--vp-c-brand-1);
  font-weight: 600;
}

.sep {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background: var(--vp-c-divider);
}
</style>
