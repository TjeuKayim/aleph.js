import { Aleph } from '../server/aleph.ts'

export const helpMessage = `
Usage:
    aleph build <dir> [...options]

<dir> represents the directory of Aleph.js app,
if the <dir> is empty, the current directory will be used.

Options:
    -L, --log-level <log-level>  Set log level [possible values: debug, info]
    -r, --reload                 Reload source code cache
    -h, --help                   Prints help message
`

export default async function (workingDir: string, options: Record<string, any>) {
  const aleph = new Aleph(workingDir, 'production', Boolean(options.r || options.reload))
  await aleph.build()
  Deno.exit(0)
}
