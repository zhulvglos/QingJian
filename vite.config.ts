import { defineConfig } from 'vite';

// 主界面与外侧快捷标签分别打包成独立页面，各自运行在原生窗口中。
export default defineConfig({
  build: {
    rollupOptions: {
      input: { main: 'index.html', quick: 'quick.html' },
    },
  },
});
