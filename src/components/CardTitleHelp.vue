<script setup lang="ts">
import { HelpCircleOutline } from '@vicons/ionicons5'

// Card header help icon: hover for a quick tooltip peek, click to pin the text
// in a dialog (mask click / X closes); show-delay keeps the tooltip from
// flashing when the pointer passes through on its way to a click /
// 卡片标题旁的问号图标：悬浮速览 tooltip，点击弹窗驻留细读（点空白或关闭按钮关闭）；
// 延迟显示避免移向图标点击时 tooltip 闪现
defineProps<{
  title: string
  help: string
}>()

const showHelp = ref(false)
</script>

<template>
  <div class="flex items-center gap-1">
    <span>{{ title }}</span>
    <NTooltip trigger="hover" :show-delay="200">
      <template #trigger>
        <NButton text size="tiny" class="text-gray-400" @click="showHelp = true">
          <template #icon>
            <NIcon size="14">
              <HelpCircleOutline />
            </NIcon>
          </template>
        </NButton>
      </template>
      <div class="whitespace-pre-line text-xs" style="max-width: 340px">
        {{ help }}
      </div>
    </NTooltip>
    <NModal v-model:show="showHelp" preset="card" :title="title" style="width: 480px">
      <div class="whitespace-pre-line text-sm">
        {{ help }}
      </div>
    </NModal>
  </div>
</template>
