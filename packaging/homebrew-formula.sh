#!/bin/sh
# Prints the Homebrew formula for a published release, for the tap
# lazymaniac/homebrew-tap (Formula/antislop.rb):
#   packaging/homebrew-formula.sh v0.2.0 > ../homebrew-tap/Formula/antislop.rb
set -eu
tag=${1:?usage: homebrew-formula.sh TAG}
repo=${ANTISLOP_REPO:-lazymaniac/antislop}
url="https://github.com/$repo/releases/download/$tag"
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
gh release download "$tag" --repo "$repo" --pattern '*.sha256' --dir "$dir"
sum() { cut -d' ' -f1 "$dir/antislop-$tag-$1.tar.gz.sha256"; }
cat <<EOF
class Antislop < Formula
  desc "Local team of AI agents working through the CLIs you already use"
  homepage "https://github.com/$repo"
  version "${tag#v}"
  license "MIT"

  on_macos do
    on_arm do
      url "$url/antislop-$tag-aarch64-apple-darwin.tar.gz"
      sha256 "$(sum aarch64-apple-darwin)"
    end
    on_intel do
      url "$url/antislop-$tag-x86_64-apple-darwin.tar.gz"
      sha256 "$(sum x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "$url/antislop-$tag-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "$(sum aarch64-unknown-linux-gnu)"
    end
    on_intel do
      url "$url/antislop-$tag-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "$(sum x86_64-unknown-linux-gnu)"
    end
  end

  def install
    bin.install "antislop"
    pkgshare.install "examples"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/antislop --version")
  end
end
EOF
