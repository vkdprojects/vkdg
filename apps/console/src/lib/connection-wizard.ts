/**
 * Data and YAML generation behind the "add connection" wizard. The gateway reads
 * connections from vkdg.yaml, so the wizard produces a snippet to paste rather
 * than calling an API.
 */

export const providerOptions = [
  { value: 'openai-compat', label: 'OpenAI-compatible (Ollama, vLLM, etc.)' },
  { value: 'anthropic-compat', label: 'Anthropic-compatible' },
  { value: 'anthropic', label: 'Anthropic (Claude)' },
  { value: 'openai', label: 'OpenAI (GPT / o-series)' },
  { value: 'groq', label: 'Groq' },
  { value: 'gemini', label: 'Google Gemini' },
  { value: 'deepseek', label: 'DeepSeek' },
  { value: 'mistral', label: 'Mistral' },
  { value: 'together', label: 'Together AI' },
  { value: 'fireworks', label: 'Fireworks AI' },
  { value: 'sambanova', label: 'SambaNova (free tier)' },
  { value: 'cerebras', label: 'Cerebras (free tier)' },
  { value: 'nvidia-nim', label: 'NVIDIA NIM' },
  { value: 'kiro', label: 'Kiro (Amazon Q)' },
];

export const defaultEnvVar: Record<string, string> = {
  anthropic: 'ANTHROPIC_API_KEY',
  openai: 'OPENAI_API_KEY',
  groq: 'GROQ_API_KEY',
  gemini: 'GEMINI_API_KEY',
  deepseek: 'DEEPSEEK_API_KEY',
  mistral: 'MISTRAL_API_KEY',
  together: 'TOGETHER_API_KEY',
  fireworks: 'FIREWORKS_API_KEY',
  sambanova: 'SAMBANOVA_API_KEY',
  cerebras: 'CEREBRAS_API_KEY',
  'nvidia-nim': 'NVIDIA_API_KEY',
  kiro: 'KIRO_API_KEY',
  'openai-compat': 'API_KEY',
};

export const defaultModels: Record<string, string> = {
  anthropic: 'claude-*',
  openai: 'gpt-*, o1-*, o3-*',
  groq: 'llama-*, mixtral-*',
  gemini: 'gemini-*',
  deepseek: 'deepseek-*',
  mistral: 'mistral-*',
  together: 'meta-llama/*',
  fireworks: 'accounts/*',
  sambanova: 'Meta-Llama-*',
  cerebras: 'llama3.1-*',
  'nvidia-nim': 'meta/llama-*',
  kiro: 'claude-*, gpt-5.6-*, minimax-*, deepseek-*, glm-*, qwen3-*, auto',
};

/** Providers that point at a user-supplied endpoint (`base_url` is only emitted for these). */
export const BASE_URL_PROVIDERS = ['openai-compat', 'anthropic-compat'];

export interface ConnectionDraft {
  provider: string;
  id: string;
  baseUrl: string;
  envVar: string;
  models: string;
}

/**
 * One entry per line with explicit indentation: `auth:` once rendered at 8
 * spaces and the gateway refused the pasted file.
 */
export function buildConnectionYaml(d: ConnectionDraft): string {
  const modelList = (d.models || '*').split(',').map((s) => `"${s.trim()}"`).join(', ');
  return [
    'connections:',
    `  - id: ${d.id || d.provider + '-default'}`,
    `    provider: ${d.provider}`,
    ...(BASE_URL_PROVIDERS.includes(d.provider) && d.baseUrl ? [`    base_url: ${d.baseUrl}`] : []),
    '    auth:',
    '      type: api_key',
    `      env_var: ${d.envVar || 'API_KEY'}`,
    `    models: [${modelList}]`,
    '    max_concurrent: 50',
    '    weight: 1',
  ].join('\n');
}
