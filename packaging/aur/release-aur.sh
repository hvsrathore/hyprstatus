#!/usr/bin/env bash
# ==============================================================================
# hyprstatus - AUR Release & Packaging Automation Script
#
# Automates:
#   1. Version extraction from Cargo.toml
#   2. Tag verification / creation
#   3. Checksum generation (sha256) for the release archive
#   4. Updating PKGBUILD with calculated sha256sum
#   5. Generating .SRCINFO via `makepkg --printsrcinfo`
#   6. Staging and committing to the Arch User Repository (AUR) git repo
#
# Usage:
#   ./packaging/aur/release-aur.sh [VERSION] [--push] [--dry-run]
# ==============================================================================

set -euo pipefail

BOLD='\033[1m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

AUR_DIR="${REPO_ROOT}/packaging/aur"
PKGBUILD_FILE="${AUR_DIR}/PKGBUILD"
SRCINFO_FILE="${AUR_DIR}/.SRCINFO"
CARGO_TOML="${REPO_ROOT}/Cargo.toml"

AUR_GIT_URL="ssh://aur@aur.archlinux.org/hyprstatus.git"

DO_PUSH=false
DRY_RUN=false
VERSION=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --push)
            DO_PUSH=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        -h|--help)
            echo -e "${BOLD}hyprstatus AUR Release Automation Script${NC}"
            echo ""
            echo "Usage:"
            echo "  $0 [VERSION] [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --push     Automatically push changes to aur.archlinux.org after committing"
            echo "  --dry-run  Compute checksums and generate files without committing to AUR"
            echo "  -h, --help Show this help message"
            exit 0
            ;;
        *)
            if [[ -z "$VERSION" ]]; then
                VERSION="$1"
            else
                echo -e "${RED}Unknown argument: $1${NC}"
                exit 1
            fi
            shift
            ;;
    esac
done

# 1. Determine version
if [[ -z "$VERSION" ]]; then
    if [[ -f "$CARGO_TOML" ]]; then
        VERSION=$(grep -m1 '^version' "$CARGO_TOML" | cut -d '"' -f2)
    fi
fi

if [[ -z "$VERSION" ]]; then
    echo -e "${RED}Error: Could not determine version from Cargo.toml. Pass it as first argument.${NC}"
    exit 1
fi

TAG="v${VERSION}"
echo -e "${BLUE}==>${NC} ${BOLD}Preparing hyprstatus release for AUR:${NC} ${GREEN}${VERSION}${NC} (tag: ${TAG})"

# 2. Update version in PKGBUILD
if [[ -f "$PKGBUILD_FILE" ]]; then
    sed -i "s/^pkgver=.*/pkgver=${VERSION}/" "$PKGBUILD_FILE"
    sed -i "s/^pkgrel=.*/pkgrel=1/" "$PKGBUILD_FILE"
    echo -e "${BLUE}==>${NC} Updated version in PKGBUILD to ${GREEN}${VERSION}${NC}"
else
    echo -e "${RED}Error: PKGBUILD not found at ${PKGBUILD_FILE}${NC}"
    exit 1
fi

# 3. Calculate SHA-256 Checksum
echo -e "${BLUE}==>${NC} Calculating SHA256 checksum..."
TEMP_TARBALL=$(mktemp --suffix=".tar.gz")
trap 'rm -f "$TEMP_TARBALL"' EXIT

TARBALL_URL="https://github.com/hvsrathore/hyprstatus/archive/refs/tags/${TAG}.tar.gz"
HTTP_STATUS=$(curl -sL -w "%{http_code}" -o "$TEMP_TARBALL" "$TARBALL_URL" || true)

if [[ "$HTTP_STATUS" == "200" && -s "$TEMP_TARBALL" ]]; then
    echo -e "${BLUE}==>${NC} Downloaded release tarball from GitHub: ${TARBALL_URL}"
else
    echo -e "${YELLOW}Notice: Remote tag archive not yet published online. Generating archive from local git tree...${NC}"
    if git rev-parse "${TAG}" >/dev/null 2>&1; then
        git -C "$REPO_ROOT" archive --format=tar.gz --prefix="hyprstatus-${VERSION}/" "${TAG}" -o "$TEMP_TARBALL"
    else
        git -C "$REPO_ROOT" archive --format=tar.gz --prefix="hyprstatus-${VERSION}/" HEAD -o "$TEMP_TARBALL"
    fi
fi

SHA256=$(sha256sum "$TEMP_TARBALL" | awk '{print $1}')
echo -e "${GREEN}==>${NC} Computed SHA256: ${BOLD}${SHA256}${NC}"

# Update PKGBUILD with calculated checksum
sed -i "s/^sha256sums=.*/sha256sums=('${SHA256}')/" "$PKGBUILD_FILE"

# 4. Generate .SRCINFO
echo -e "${BLUE}==>${NC} Generating .SRCINFO with makepkg..."
if command -v makepkg >/dev/null 2>&1; then
    (cd "$AUR_DIR" && makepkg --printsrcinfo > .SRCINFO)
    echo -e "${GREEN}==>${NC} Generated .SRCINFO successfully"
else
    echo -e "${RED}Error: makepkg command not found in PATH.${NC}"
    exit 1
fi

# 5. Check Dry-Run
if [[ "$DRY_RUN" == true ]]; then
    echo -e "${YELLOW}==> Dry-run mode enabled. PKGBUILD and .SRCINFO are updated. Skipping AUR git push.${NC}"
    exit 0
fi

# 6. AUR Git Submission
echo -e "${BLUE}==>${NC} Preparing AUR git clone/stage..."
AUR_WORK_DIR="${REPO_ROOT}/target/aur-pkg"
mkdir -p "$(dirname "$AUR_WORK_DIR")"

if [[ ! -d "${AUR_WORK_DIR}/.git" ]]; then
    echo -e "${BLUE}==>${NC} Cloning AUR repository into ${AUR_WORK_DIR}..."
    if ! git clone "$AUR_GIT_URL" "$AUR_WORK_DIR" 2>/dev/null; then
        echo -e "${YELLOW}Notice: Could not clone ${AUR_GIT_URL} directly (AUR repo may need to be initialized or SSH key setup).${NC}"
        echo -e "${YELLOW}Initializing local AUR staging repository...${NC}"
        mkdir -p "$AUR_WORK_DIR"
        (
            cd "$AUR_WORK_DIR"
            git init
            git remote add origin "$AUR_GIT_URL" || true
        )
    fi
else
    echo -e "${BLUE}==>${NC} Pulling latest changes from AUR..."
    (cd "$AUR_WORK_DIR" && git pull origin master --rebase 2>/dev/null || true)
fi

# Copy PKGBUILD and .SRCINFO to AUR repository
cp "$PKGBUILD_FILE" "${AUR_WORK_DIR}/PKGBUILD"
cp "$SRCINFO_FILE" "${AUR_WORK_DIR}/.SRCINFO"

(
    cd "$AUR_WORK_DIR"
    git add PKGBUILD .SRCINFO
    
    if git diff --staged --quiet; then
        echo -e "${YELLOW}==> No changes detected in AUR repository (already up to date).${NC}"
    else
        echo -e "${GREEN}==> Changes staged for AUR submission:${NC}"
        git diff --staged --stat
        git commit -m "Update hyprstatus to v${VERSION}"
        echo -e "${GREEN}==> Committed release v${VERSION} to local AUR staging repo.${NC}"
        
        if [[ "$DO_PUSH" == true ]]; then
            echo -e "${BLUE}==>${NC} Pushing to AUR remote (${AUR_GIT_URL})..."
            git push origin master
            echo -e "${GREEN}==> Successfully published hyprstatus v${VERSION} to AUR!${NC}"
        else
            echo -e "${YELLOW}To push to AUR run:${NC}"
            echo -e "  cd ${AUR_WORK_DIR} && git push origin master"
        fi
    fi
)

echo -e "\n${GREEN}${BOLD}✓ AUR package preparation complete for hyprstatus v${VERSION}!${NC}\n"
