const OpenAI = require("openai");
const Anthropic = require("@anthropic-ai/sdk");
const openaiPkg = require("openai/package.json");
const anthropicPkg = require("@anthropic-ai/sdk/package.json");

const openai = new OpenAI({
  apiKey: "test-key",
  baseURL: "https://example.invalid/v1"
});
const anthropic = new Anthropic({
  apiKey: "test-key",
  baseURL: "https://example.invalid"
});

console.log([
  "openai",
  openaiPkg.version,
  typeof OpenAI,
  typeof OpenAI.OpenAI,
  typeof openai.chat.completions.create,
  typeof openai.responses,
  "anthropic",
  anthropicPkg.version,
  typeof Anthropic,
  typeof Anthropic.Anthropic,
  typeof anthropic.messages.create,
  typeof anthropic.beta,
  typeof fetch,
  typeof Headers,
  typeof Request,
  typeof Response,
  typeof AbortController
].join(":"));
