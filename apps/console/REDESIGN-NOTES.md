# Notas do redesign do console — Fase 1

## 1. Relatório: o que existe de verdade sobre "limites das contas conectadas"

Investigado: `crates/vkdg-admin/src/handlers/connections.rs`, `crates/vkdg-admin/src/handlers/keys.rs`,
`crates/vkdg-connections/src/quota.rs`, `crates/vkdg-http/src/{app_state,auth,pipeline/*}.rs`,
`crates/vkdg-governance/src/store.rs`, `plugins/providers/kiro/src/decode.rs`, `apps/console/src/lib/api.ts`.

Legenda: **[pronto]** já sai da admin API hoje, front pode consumir sem mexer no backend.
**[precisa backend]** o dado existe em algum crate do core mas a admin API não expõe (endpoint novo ou campo novo).
**[upstream não fornece]** nenhuma camada tem o dado de forma confiável — não dá pra fabricar isso na UI.

### 1.1 Por conexão (`ConnectionSummary`, `GET /admin/v1/connections`)

- **[pronto]** `status` (`healthy` / `degraded` / `circuit_open` / `cooldown` / `unknown`) — vem do `catalog` em memória (`connections.rs:58-73`).
- **[pronto]** `model_count`, `active_requests`, `max_concurrent` — contagem real de modelos da conexão e slots concorrentes (`conn.models.len()`, `live.active_requests()`, `conn.max_concurrent`).
- **[pronto]** `cooldown_until` (RFC 3339) e `failure_count` — só presentes quando `status` é `cooldown`/`circuit_open`; vêm do `ConnectionState` real do circuit breaker.
- **[upstream não fornece]** "créditos restantes" / "quota do plano Kiro". O decoder do Kiro (`plugins/providers/kiro/src/decode.rs:107-116`) recebe o `meteringEvent` (que é onde a AWS/Kiro mandaria créditos) e **descarta explicitamente**: o comentário no código diz literalmente `meteringEvent (credits), ... carry nothing to forward`. Não existe, hoje, nenhum ponto do pipeline Kiro que leia créditos/quota do provedor. Mostrar "X% do plano Kiro usado" seria inventado.
- **[upstream não fornece hoje / parcialmente instrumentado]** `quota_headroom` por conexão. Existe um `QuotaTracker` (`crates/vkdg-connections/src/quota.rs`) com `record_usage`/`set_limit`/`headroom`, e ele **é** chamado em produção:
  - `record_usage` é chamado em `vkdg-http/src/pipeline/phases.rs:208-215`, mas com tokens **estimados** por `response_body.len() / 4` (aproximação grosseira de bytes→tokens, não a contagem real do provedor).
  - `set_limit` **nunca é chamado fora dos testes** (`quota.rs` `mod tests`). Sem `set_limit`, o `QuotaWindow.limit` fica `None` para sempre e `headroom()` retorna `None` sempre (`quota.rs:82-85`, `QuotaWindow::headroom` linha 29-33: `let limit = self.limit?`).
  - Resultado prático: `RoutingHints.quota_headroom` (usado pelo *scorer* de rotas, `vkdg-connections/src/types.rs:113-114`) hoje é sempre vazio/`None` para todas as conexões reais. O mecanismo existe, mas está "desligado" — não há nenhuma origem de limite real (nem config, nem header do provedor) alimentando `set_limit`.
  - Conclusão honesta: **não dá para mostrar "quota usada" nem "headroom" na conexão hoje**, porque o único número que existiria seria sempre vazio. Mostrar isso na UI seria mostrar um campo permanentemente "sem dado".

### 1.2 Por chave de cliente (`ClientKey`, `GET /admin/v1/keys`)

- **[pronto]** `usage_this_month: { input_tokens, output_tokens, requests }` — isso É real e persistido: `VirtualKeyStore::record_usage` (`vkdg-governance/src/store.rs`) é chamado a cada resposta autenticada em `vkdg-http/src/auth.rs:337-340`, com os tokens **reportados pelo provedor** (`usage.input`/`usage.output`, que vêm do `ConversationEvent::Usage` — ver 1.3). Isso é o dado de uso mais confiável que o sistema tem hoje, granular por chave e por mês corrente (UTC).
- **[pronto]** `monthly_token_limit` e `requests_per_minute` — limites configuráveis pelo operador na própria chave (`KeyPatch`/`CreateKeyBody`), aplicados de verdade na autenticação (ver teste `record_usage` + `monthly_token_limit` em `auth.rs` linha ~660). Ou seja: **dá pra montar um medidor real "uso do mês vs. limite configurado"** por chave, quando o limite existir; quando `monthly_token_limit` for `null`, o painel deve mostrar "sem limite configurado", nunca inventar um teto.
- **[pronto]** `expires_at`, `allowed_models`, `allowed_ips`, `no_log`, `status` (`active`/`disabled`/`expired`/`revoked`) — tudo já no tipo `ClientKey` do `api.ts`.
- **[precisa backend]** série histórica (uso por dia/semana). Hoje só existe o agregado do mês corrente; não há endpoint de série temporal. Um gráfico de tendência exigiria um novo endpoint (`/admin/v1/keys/{id}/usage?range=...`) e provavelmente uma tabela de agregados diários no `vkdg-governance`.

### 1.3 Tokens/custo por requisição (`RequestSummary`, `GET /admin/v1/requests`)

- **[pronto]** `input_tokens`, `output_tokens` — "absent while streaming or when not reported" (comentário no próprio tipo). Isto é honesto: quando o provedor manda usage real (`ConversationEvent::Usage` reportado), esses campos vêm preenchidos; quando não manda, ficam `null` — a UI já deve tratar ausência como ausência, não como zero.
- **[pronto]** `cost_microdollars` — "at the provider's list price; absent when it lists none". Mesma lógica: alguns provedores não têm tabela de preço conhecida, e o campo fica ausente. Não inventar custo quando ausente.
- **[upstream não fornece, caso Kiro]** Especificamente para Kiro, o `usage()` do decoder (`decode.rs:278-298`) **estima** quando o provedor não manda números reais: `output ≈ chars/4`, `input` derivado de `contextUsagePercentage × 200_000`. Esses valores chegam ao pipeline marcados internamente como `UsageCount::Estimated` vs `UsageCount::Reported` (`vkdg_operations::UsageCount`), mas **essa distinção reported/estimated não é exposta na admin API** — `RequestSummary.input_tokens`/`output_tokens` no `api.ts` são só `number | null`, sem indicar se é estimativa. **[precisa backend]** se quisermos que a UI rotule "estimado" vs "reportado pelo provedor" (recomendado para não parecer que estamos garantindo precisão que não existe), a admin API precisa carregar esse enum.

### 1.4 Contas OAuth conectadas (`Account`, `GET /admin/v1/accounts`)

- **[pronto]** `status` (`active`/`needs_login`), `expires_at` do token OAuth, `has_refresh_token`, `revoked_reason`. Isso dá um painel real de "saúde da credencial" (token válido, precisa relogar, tem refresh automático ou não).
- **[upstream não fornece]** Não existe nenhum campo de "créditos do plano" ou "quota da conta" na struct `Account`, porque a Amazon/Kiro não devolve isso em nenhum endpoint que o VKDG chama hoje (reforça o achado do item 1.1: o `meteringEvent` é descartado no decoder).

### 1.5 O que dá pra construir HOJE (sem mexer no backend) — painel honesto

1. **Grid de conexões** (já existe, repaginar): status real, `model_count`, `active_requests`/`max_concurrent` como medidor de concorrência (isso é 100% real e é o "limite" mais tangível que existe hoje — não é quota de créditos, é capacidade concorrente), cooldown/circuit com tempo até liberar, contagem de falhas.
2. **Uso do mês por chave**: `usage_this_month.input_tokens/output_tokens/requests` versus `monthly_token_limit` (quando configurado) — medidor de progresso real. Sem limite configurado, mostrar "sem limite" em vez de barra vazia.
3. **Requests recentes**: tokens/custo por request quando presentes, com estado vazio explícito quando ausentes (não renderizar "0").
4. **Saúde de contas OAuth**: expiração do token, precisa reautenticar, tem refresh automático.

O que **não** vamos construir nesta fase por não ser honesto com o dado disponível:
- Barra de "quota/crédito restante" por conexão ou por conta Kiro (upstream não fornece; `QuotaTracker.set_limit` não é chamado em produção).
- Distinção visual "estimado vs. reportado" nos tokens de request (precisa backend expor o enum `UsageCount`).
- Gráfico histórico de uso por chave (precisa backend: endpoint de série temporal).

## 2. Fundação de design (Fase 1)

Direção: instrumentação de infraestrutura (Linear/Vercel/Grafana/Datadog), não "AI product". Sem gradiente roxo,
sem glow, sem sparkles. Dark, alto contraste nos dados, tipografia mono para números/IDs, acento teal único,
bordas finas, cantos pequenos, sem sombra pesada. Estado nunca só por cor (texto/ícone sempre junto).

Arquivos alterados nesta fase: ver seção 3 do relatório enviado ao diretor (resumo da conversa).
