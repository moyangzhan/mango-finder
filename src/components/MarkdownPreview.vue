<script setup lang="ts">
import { openUrl } from '@tauri-apps/plugin-opener'
import { renderMarkdown } from '@/utils/markdown'

const props = defineProps<{
  content: string
}>()

const rendered = ref('')

watch(() => props.content, async (val) => {
  rendered.value = val ? await renderMarkdown(val) : ''
}, { immediate: true })

// v-html anchors would navigate the whole webview away; http(s) links go to the system browser, anything else (relative/mailto/...) is swallowed /
// v-html 里的链接会把整个 webview 导航走；http(s) 外链交给系统浏览器打开，其余链接一律拦截
function onClick(e: MouseEvent) {
  const anchor = (e.target as HTMLElement).closest('a')
  if (!anchor)
    return
  e.preventDefault()
  const href = anchor.getAttribute('href') ?? ''
  if (/^https?:\/\//i.test(href))
    openUrl(href).catch(err => console.error('openUrl failed:', err))
}
</script>

<template>
  <div v-if="rendered" class="markdown-body" @click="onClick" v-html="rendered" />
  <div v-else>
    <slot name="empty">
      {{ $t('common.noData') }}
    </slot>
  </div>
</template>

<style scoped>
/* v-html content carries no scope attribute, so descendant selectors must use :deep() /
   v-html 内容不带 scope 属性，后代选择器必须用 :deep() 才能生效 */
.markdown-body {
  font-size: 14px;
  line-height: 1.6;
  word-wrap: break-word;
}

.markdown-body :deep(h1),
.markdown-body :deep(h2),
.markdown-body :deep(h3),
.markdown-body :deep(h4) {
  margin-top: 1em;
  margin-bottom: 0.5em;
  font-weight: 600;
}

.markdown-body :deep(h1) { font-size: 1.5em; }
.markdown-body :deep(h2) { font-size: 1.3em; }
.markdown-body :deep(h3) { font-size: 1.15em; }

.markdown-body :deep(p) {
  margin: 0.5em 0;
}

/* Tailwind preflight strips list markers and indent — restore them /
   Tailwind preflight 清空了列表符号与缩进，需显式恢复 */
.markdown-body :deep(ul) { list-style: disc; }
.markdown-body :deep(ol) { list-style: decimal; }

.markdown-body :deep(ul),
.markdown-body :deep(ol) {
  padding-left: 1.5em;
  margin: 0.5em 0;
}

.markdown-body :deep(li) {
  margin: 0.25em 0;
}

.markdown-body :deep(li > ul),
.markdown-body :deep(li > ol) {
  margin: 0.25em 0;
}

.markdown-body :deep(a) {
  color: var(--primary-color);
}

.markdown-body :deep(img) {
  max-width: 100%;
  height: auto;
  border-radius: 4px;
  margin: 0.5em 0;
}

.markdown-body :deep(hr) {
  border: none;
  border-top: 1px solid var(--border-color);
  margin: 1em 0;
}

.markdown-body :deep(table) {
  border-collapse: collapse;
  width: 100%;
  margin: 0.5em 0;
}

.markdown-body :deep(th),
.markdown-body :deep(td) {
  border: 1px solid var(--border-color);
  padding: 6px 12px;
  text-align: left;
}

.markdown-body :deep(th) {
  background: rgba(127, 127, 127, 0.08);
}

.markdown-body :deep(code) {
  background: rgba(127, 127, 127, 0.1);
  padding: 0.2em 0.4em;
  border-radius: 3px;
  font-size: 0.9em;
}

.markdown-body :deep(pre) {
  background: rgba(127, 127, 127, 0.1);
  padding: 1em;
  border-radius: 4px;
  overflow-x: auto;
}

.markdown-body :deep(pre code) {
  background: none;
  padding: 0;
}
</style>
