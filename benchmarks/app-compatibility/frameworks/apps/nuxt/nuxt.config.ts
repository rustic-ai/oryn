export default defineNuxtConfig({
  ssr: true,
  nitro: {preset: 'static'},
  app: {baseURL: '/app-compatibility/frameworks/nuxt/'},
  experimental: {appManifest: false},
  vite: {build: {minify: false}},
  compatibilityDate: '2026-08-27',
});
