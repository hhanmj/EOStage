# Run a bash command in the Linux probe container against an EOStage worktree.
# Build the image once:  docker build -t eostage-linux-probe scripts/linux
# Usage: run_probe.ps1 -Worktree <checkout> -Out <output dir> -Cmd "<bash command run in /src>"
# Full check:            run_probe.ps1 -Worktree . -Out <dir> -Cmd "bash scripts/linux/full_test.sh"
# The worktree is mounted read-only and copied into the container; results land in <Out>.
param(
    [Parameter(Mandatory)][string]$Worktree,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$Cmd
)
$wt = (Resolve-Path $Worktree).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$outDir = (Resolve-Path $Out).Path
# One build cache and scratch volume per worktree, so parallel runs never share a target lock.
$suffix = "-" + (Split-Path $wt -Leaf)
$prep = "mkdir -p /src && tar -C /wt --exclude=./target --exclude=./ui/node_modules --exclude=./.git -cf - . | tar -C /src -xf - && cd /src && export TMPDIR=/work HDF5_PLUGIN_PRELOAD='::' CARGO_TERM_COLOR=never && "
docker run --rm -v "${wt}:/wt:ro" -v "${outDir}:/out" -v eostage-cargo-registry:/root/.cargo/registry -v "eostage-target${suffix}:/src/target" -v "eostage-work${suffix}:/work" -v "eostage-node-modules${suffix}:/src/ui/node_modules" eostage-linux-probe bash -c ($prep + $Cmd)
