import type { App } from 'vue'
import { createI18n } from 'vue-i18n'
import enUS from './en-US'
import zhCN from './zh-CN'

export type Language = 'zh-CN' | 'en-US'
const defaultLocale = 'en-US'

const i18n = createI18n({
  locale: defaultLocale,
  fallbackLocale: 'en-US',
  allowComposition: true,
  messages: {
    'en-US': enUS,
    'zh-CN': zhCN,
  },
})

export const t = i18n.global.t

export function setLocale(locale: Language) {
  i18n.global.locale = locale
}

export function getLocale(): Language {
  return i18n.global.locale as Language
}

export function setupI18n(app: App) {
  app.use(i18n)
}

export default i18n
