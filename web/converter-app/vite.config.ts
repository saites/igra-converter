import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const hostname = "http://192.168.4.121:8080"
// const hostname = "http://192.168.175.28:8080"

// https://vitejs.dev/config/
export default defineConfig({
  base: "/",
  appType: "mpa",
  server: {
    host: "0.0.0.0",
    origin: "http://localhost:8081",
    proxy: {
      '/validate': hostname,
      '/generate': hostname,
      '/search': hostname,
    },
    headers: {
      "X-Content-Type-Options": ["nosniff"],
    },
  },
  plugins: [
    vue(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    }
  },
  css: {
    postcss: {
      plugins: [require("tailwindcss"), require("autoprefixer")],
    },
  },
})
