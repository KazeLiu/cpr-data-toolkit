import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import CodexProxyUI from '@codex-proxy/ui/vite'
import { defineConfig } from 'vite'

export default defineConfig({
  base: './',
  plugins: [
    vue(),
    tailwindcss(),
    CodexProxyUI(),
    {
      name: 'plugin-classic-script',
      transformIndexHtml: {
        order: 'post',
        handler: (html) => html.replace('type="module" crossorigin', 'defer'),
      },
    },
  ],
  build: {
    outDir: 'dist',
    modulePreload: false,
    cssCodeSplit: false,
    rolldownOptions: {
      output: {
        format: 'iife',
        codeSplitting: false,
        entryFileNames: 'app.js',
        assetFileNames: 'app.[ext]',
      },
    },
  },
})
