import { defineConfig } from 'vitepress'

const repo = 'https://github.com/QaidVoid/smudge'

export default defineConfig({
  title: 'Smudge',
  description:
    'A native Rust GUI library with a retained widget tree, fine-grained reactive signals and damage-tracked repainting.',
  lang: 'en-US',
  cleanUrls: true,
  lastUpdated: true,
  appearance: 'dark',

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' }],
    ['meta', { name: 'theme-color', content: '#101115' }],
    ['meta', { property: 'og:type', content: 'website' }],
    ['meta', { property: 'og:title', content: 'Smudge' }],
    [
      'meta',
      {
        property: 'og:description',
        content: 'A Rust GUI library that repaints only what changed.',
      },
    ],
  ],

  markdown: {
    theme: { light: 'github-dark', dark: 'github-dark' },
  },

  themeConfig: {
    logo: '/logo.svg',

    nav: [
      { text: 'Guide', link: '/guide/introduction', activeMatch: '/guide/' },
      { text: 'Reference', link: '/reference/crates', activeMatch: '/reference/' },
      { text: 'Internals', link: '/internals/frame', activeMatch: '/internals/' },
      { text: 'API', link: 'https://docs.rs/smudge' },
    ],

    sidebar: {
      '/': [
        {
          text: 'Start',
          items: [
            { text: 'Introduction', link: '/guide/introduction' },
            { text: 'Quick start', link: '/guide/quick-start' },
          ],
        },
        {
          text: 'Building interfaces',
          items: [
            { text: 'Signals', link: '/guide/signals' },
            { text: 'Elements and layout', link: '/guide/layout' },
            { text: 'Styling and themes', link: '/guide/styling' },
            { text: 'Widgets', link: '/guide/widgets' },
            { text: 'Text and input', link: '/guide/text-and-input' },
            { text: 'Dynamic content', link: '/guide/dynamic-content' },
          ],
        },
        {
          text: 'Running',
          items: [
            { text: 'Windows and renderers', link: '/guide/rendering' },
            { text: 'On the web', link: '/guide/web' },
            { text: 'Accessibility', link: '/guide/accessibility' },
            { text: 'Testing headless', link: '/guide/testing' },
          ],
        },
        {
          text: 'Reference',
          items: [
            { text: 'Crates', link: '/reference/crates' },
            { text: 'Cargo features', link: '/reference/features' },
            { text: 'Known limits', link: '/reference/limits' },
          ],
        },
        {
          text: 'Internals',
          items: [
            { text: 'Anatomy of a frame', link: '/internals/frame' },
            { text: 'Performance', link: '/internals/performance' },
          ],
        },
      ],
    },

    socialLinks: [{ icon: 'github', link: repo }],

    editLink: {
      pattern: `${repo}/edit/main/docs/:path`,
      text: 'Edit this page',
    },

    search: { provider: 'local' },

    outline: { level: [2, 3], label: 'On this page' },

    footer: {
      message: 'Released under the MIT or Apache 2.0 license, at your option.',
    },
  },
})
