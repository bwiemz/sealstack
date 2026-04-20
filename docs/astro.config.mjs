import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
  site: 'https://signet.dev',
  integrations: [
    starlight({
      title: 'Signet',
      description:
        'Open-source, enterprise-grade context operating system. MCP-native, Rust-backed.',
      social: {
        github: 'https://github.com/bwiemz/signet'
      },
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Getting started', slug: 'getting-started' },
            { label: 'Architecture overview', slug: 'architecture' }
          ]
        },
        {
          label: 'CSL — Context Schema Language',
          items: [
            { label: 'Tutorial', slug: 'csl/tutorial' },
            { label: 'Reference', slug: 'csl/reference' }
          ]
        },
        {
          label: 'Operations',
          items: [
            { label: 'Deployment', slug: 'ops/deployment' },
            { label: 'Configuration', slug: 'ops/configuration' },
            { label: 'Multi-tenant isolation', slug: 'ops/multi-tenant' }
          ]
        },
        {
          label: 'APIs',
          items: [
            { label: 'REST reference', slug: 'api/rest' },
            { label: 'MCP tools', slug: 'api/mcp' }
          ]
        }
      ]
    })
  ]
});
