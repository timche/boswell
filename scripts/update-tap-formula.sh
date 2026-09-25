#!/bin/sh
# Rewrite the Homebrew formula in timche/homebrew-tap for a released tag and push it.
#
# The whole formula is generated rather than patched line by line, so rerunning
# this for a tag that is already in the tap produces no diff and no commit.
# Pushing needs a deploy key on the tap: the caller puts it in GIT_SSH_COMMAND.
# The formula carries no `version`, which brew audit rejects as redundant with
# the one it scans out of the release URL.
set -eu

tag=${1:-}
if [ -z "$tag" ]; then
  echo "usage: $0 <tag>" >&2
  exit 2
fi
version=${tag#v}

repo=timche/boswell
tap_remote=${TAP_REMOTE:-git@github.com:timche/homebrew-tap.git}
darwin=boswell-aarch64-apple-darwin.tar.gz
linux=boswell-x86_64-unknown-linux-musl.tar.gz

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

gh release download "$tag" --repo "$repo" --pattern '*.sha256' --dir "$work"
darwin_sha=$(awk '{print $1}' "$work/$darwin.sha256")
linux_sha=$(awk '{print $1}' "$work/$linux.sha256")

git clone --depth 1 "$tap_remote" "$work/tap"

cat > "$work/tap/Formula/boswell.rb" <<FORMULA
class Boswell < Formula
  desc "Daemon that watches git repositories and commits and pushes what changes"
  homepage "https://github.com/$repo"

  on_macos do
    on_arm do
      url "https://github.com/$repo/releases/download/$tag/$darwin"
      sha256 "$darwin_sha"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/$repo/releases/download/$tag/$linux"
      sha256 "$linux_sha"
    end
  end

  def install
    bin.install "boswell"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/boswell --version")
  end
end
FORMULA

cd "$work/tap"
if git diff --quiet -- Formula/boswell.rb; then
  echo "the tap is already at $version with these checksums; nothing to push"
  exit 0
fi

git -c user.name="github-actions[bot]" \
    -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
    commit -q -m "boswell $version" -- Formula/boswell.rb
git push -q origin HEAD:main
echo "pushed boswell $version to the tap"
