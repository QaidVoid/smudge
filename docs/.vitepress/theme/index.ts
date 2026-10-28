import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'

import '@fontsource-variable/space-grotesk'
import '@fontsource-variable/figtree'
import '@fontsource-variable/geist-mono'
import './style.css'

import DamageDemo from './components/DamageDemo.vue'
import Home from './components/Home.vue'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('Home', Home)
    app.component('DamageDemo', DamageDemo)
  },
} satisfies Theme
