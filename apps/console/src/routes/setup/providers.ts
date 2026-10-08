import { m } from '$lib/paraglide/messages.js';

export interface SetupProvider {
  id: string;
  label: string;
  desc: string;
  /** Where to create an API key; empty for a custom endpoint. */
  url: string;
  free?: boolean;
}

export function setupProviders(): SetupProvider[] {
  return [
    { id: 'anthropic', label: 'Anthropic',              desc: m.setup_provider_api_key(),     url: 'https://console.anthropic.com/settings/keys' },
    { id: 'openai',    label: 'OpenAI',                 desc: m.setup_provider_api_key(),     url: 'https://platform.openai.com/api-keys' },
    { id: 'groq',      label: 'Groq',                   desc: m.setup_provider_free_tier(),   url: 'https://console.groq.com/keys', free: true },
    { id: 'gemini',    label: 'Google Gemini',          desc: m.setup_provider_api_key(),     url: 'https://aistudio.google.com/app/apikey' },
    { id: 'deepseek',  label: 'DeepSeek',               desc: m.setup_provider_api_key(),     url: 'https://platform.deepseek.com/api_keys' },
    { id: 'mistral',   label: 'Mistral',                desc: m.setup_provider_api_key(),     url: 'https://console.mistral.ai/api-keys/' },
    { id: 'kiro',      label: 'Kiro (Amazon Q)',        desc: m.setup_provider_api_key(),     url: 'https://kiro.dev/' },
    { id: 'custom',    label: 'Custom (OpenAI-compat)', desc: m.setup_provider_custom_desc(), url: '' },
  ];
}
