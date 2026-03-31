#!/usr/bin/env python3
"""Validate Gestura's release definition and emitted release artifacts."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
from collections import OrderedDict

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFINITION_PATH = ROOT / "release" / "release-definition.json"
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
ALLOWED_CHANNEL_STATUSES = {"published", "documented-manual", "planned"}


def fail(message: str) -> "NoReturn":
    raise SystemExit(message)


def load_json(path: pathlib.Path) -> dict:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def load_definition() -> dict:
    definition = load_json(DEFINITION_PATH)
    if definition.get("schema_version") != 1:
        fail(f"Unsupported release definition schema: {definition.get('schema_version')}")
    return definition


def load_tauri_config() -> dict:
    return load_json(ROOT / "crates" / "gestura-gui" / "tauri.conf.json")


def validate_definition(definition: dict) -> None:
    release = definition.get("release")
    platforms = definition.get("platforms")
    channels = definition.get("channels")
    if not isinstance(release, dict) or not isinstance(platforms, dict) or not isinstance(channels, dict):
        fail("Release definition must contain release/platforms/channels objects")
    if release.get("tag_prefix") != "v":
        fail("Release definition tag_prefix must remain 'v' for the current tag workflow")
    channel_names = set(channels)
    for platform_name, platform in platforms.items():
        if not isinstance(platform, dict):
            fail(f"Platform definition for {platform_name} must be an object")
        for key in ("architectures", "features", "required_assets", "channels"):
            value = platform.get(key)
            if not isinstance(value, list) or not value or not all(isinstance(item, str) and item for item in value):
                fail(f"Platform {platform_name} must define a non-empty string list for {key}")
        unknown_channels = sorted(set(platform["channels"]) - channel_names)
        if unknown_channels:
            fail(f"Platform {platform_name} references unknown channels: {unknown_channels}")
    for channel_name, channel in channels.items():
        if not isinstance(channel, dict):
            fail(f"Channel definition for {channel_name} must be an object")
        if channel.get("status") not in ALLOWED_CHANNEL_STATUSES:
            fail(f"Channel {channel_name} has unsupported status: {channel.get('status')}")
        assets = channel.get("required_assets")
        if not isinstance(assets, list) or not assets or not all(isinstance(item, str) and item for item in assets):
            fail(f"Channel {channel_name} must define a non-empty string list for required_assets")


def load_versions() -> dict[str, str]:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    workspace_version = cargo["workspace"]["package"]["version"]
    tauri_version = load_json(ROOT / "crates" / "gestura-gui" / "tauri.conf.json")["version"]
    frontend_version = load_json(ROOT / "crates" / "gestura-gui" / "frontend" / "package.json")["version"]
    return {
        "Cargo.toml": workspace_version,
        "crates/gestura-gui/tauri.conf.json": tauri_version,
        "crates/gestura-gui/frontend/package.json": frontend_version,
    }


def validate_versions() -> str:
    versions = load_versions()
    unique_versions = set(versions.values())
    if len(unique_versions) != 1:
        fail(f"Version drift detected across release metadata: {versions}")
    version = unique_versions.pop()
    if not SEMVER_RE.fullmatch(version):
        fail(f"Workspace version is not valid semver: {version}")
    return version


def render(pattern: str, tag: str) -> str:
    return pattern.format(tag=tag, version=tag.removeprefix("v"))


def ordered_unique(values: list[str]) -> list[str]:
    return list(OrderedDict((value, None) for value in values))


def expected_assets(definition: dict, tag: str, *, include_manifest_asset: bool = True) -> dict[str, list[str]]:
    platform_assets = {
        platform_name: [render(pattern, tag) for pattern in platform["required_assets"]]
        for platform_name, platform in definition["platforms"].items()
    }
    manifest_asset = render(definition["release"]["manifest_asset"], tag)
    channel_assets: dict[str, list[str]] = {}
    for channel_name, channel in definition["channels"].items():
        if channel["status"] == "planned":
            continue
        assets = [render(pattern, tag) for pattern in channel["required_assets"]]
        if not include_manifest_asset:
            assets = [asset for asset in assets if asset != manifest_asset]
        channel_assets[channel_name] = assets
    return {"platforms": platform_assets, "channels": channel_assets}


def assert_assets_exist(expected: dict[str, list[str]], dist_dir: pathlib.Path) -> None:
    missing: list[str] = []
    for asset in [*expected["platforms"].values(), *expected["channels"].values()]:
        for filename in asset:
            if not (dist_dir / filename).is_file():
                missing.append(filename)
    if missing:
        fail(f"Missing required release assets in {dist_dir}: {sorted(set(missing))}")


def write_outputs(path: pathlib.Path, values: dict[str, str]) -> None:
    with path.open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def normalize_dependency_text(value: str) -> str:
    value = re.sub(r"\s*\|\s*", " | ", value.strip())
    return re.sub(r"\s+", " ", value)


def configured_linux_dependencies() -> dict[str, list[str]]:
    tauri_config = load_tauri_config()
    linux_bundle = tauri_config["bundle"]["linux"]
    return {
        "deb": [normalize_dependency_text(dep) for dep in linux_bundle["deb"].get("depends", [])],
        "rpm": [normalize_dependency_text(dep) for dep in linux_bundle["rpm"].get("depends", [])],
    }


def validate_linux_package_metadata(deb_path: pathlib.Path, rpm_path: pathlib.Path) -> dict[str, list[str]]:
    expected = configured_linux_dependencies()

    deb_raw = subprocess.check_output(["dpkg-deb", "-f", str(deb_path), "Depends"], cwd=ROOT, text=True).strip()
    deb_actual = [normalize_dependency_text(dep) for dep in deb_raw.replace("\n", " ").split(",") if dep.strip()]
    missing_deb = [dep for dep in expected["deb"] if dep not in deb_actual]
    if missing_deb:
        fail(
            "DEB package metadata is missing configured runtime dependencies: "
            f"{missing_deb}. Actual Depends: {deb_actual}"
        )

    rpm_raw = subprocess.check_output(["rpm", "-qp", "--requires", str(rpm_path)], cwd=ROOT, text=True)
    rpm_actual = [normalize_dependency_text(line) for line in rpm_raw.splitlines() if line.strip()]
    missing_rpm = [
        dep
        for dep in expected["rpm"]
        if not any(line == dep or line.startswith(f"{dep} ") for line in rpm_actual)
    ]
    if missing_rpm:
        fail(
            "RPM package metadata is missing configured runtime dependencies: "
            f"{missing_rpm}. Actual Requires lines: {rpm_actual}"
        )

    return {
        "expected_deb": expected["deb"],
        "actual_deb": deb_actual,
        "expected_rpm": expected["rpm"],
        "actual_rpm": rpm_actual,
    }


def metadata_command(args: argparse.Namespace) -> int:
    definition = load_definition()
    validate_definition(definition)
    version = validate_versions()
    expected_tag = f"{definition['release']['tag_prefix']}{version}"
    event_name = os.environ.get("EVENT_NAME", "workflow_dispatch")
    input_publish = os.environ.get("INPUT_PUBLISH", "false").lower() == "true"
    ref_name = os.environ.get("REF_NAME", "")
    if event_name == "push" and ref_name and ref_name != expected_tag:
        fail(f"Tag/version mismatch: workflow triggered for {ref_name}, but repository version resolves to {expected_tag}")
    values = {
        "publish": str(event_name == "push" or input_publish).lower(),
        "prerelease": "true" if "-" in version else "false",
        "release_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "tag": expected_tag,
        "version": version,
        "macos_features": ",".join(definition["platforms"]["macos"]["features"]),
        "linux_features": ",".join(definition["platforms"]["linux"]["features"]),
        "windows_features": ",".join(definition["platforms"]["windows"]["features"]),
    }
    if args.github_output:
        write_outputs(pathlib.Path(args.github_output), values)
    else:
        print(json.dumps(values, indent=2))
    return 0


def artifacts_command(args: argparse.Namespace) -> int:
    definition = load_definition()
    validate_definition(definition)
    expected = expected_assets(definition, args.tag)
    dist_dir = pathlib.Path(args.dist_dir)
    assert_assets_exist(expected, dist_dir)
    if args.summary_file:
        lines = [
            "## Release artifact completeness",
            "",
            f"Validated `{args.tag}` against `release/release-definition.json`.",
            "",
            "### Platform assets",
        ]
        for platform_name, assets in expected["platforms"].items():
            lines.append(f"- `{platform_name}`: {', '.join(f'`{asset}`' for asset in assets)}")
        lines.append("")
        lines.append("### Channels")
        for channel_name, assets in expected["channels"].items():
            status = definition["channels"][channel_name]["status"]
            lines.append(f"- `{channel_name}` ({status}): {', '.join(f'`{asset}`' for asset in assets)}")
        pathlib.Path(args.summary_file).write_text("\n".join(lines) + "\n", encoding="utf-8")
    return 0


def manifest_command(args: argparse.Namespace) -> int:
    definition = load_definition()
    validate_definition(definition)
    expected = expected_assets(definition, args.tag, include_manifest_asset=False)
    dist_dir = pathlib.Path(args.dist_dir)
    assert_assets_exist(expected, dist_dir)
    files = sorted(path.name for path in dist_dir.iterdir() if path.is_file())
    manifest = {
        "schema_version": 1,
        "tag": args.tag,
        "version": args.tag.removeprefix("v"),
        "release_definition": str(DEFINITION_PATH.relative_to(ROOT)),
        "artifacts": files,
        "platforms": {
            platform_name: {
                "architectures": definition["platforms"][platform_name]["architectures"],
                "features": definition["platforms"][platform_name]["features"],
                "channels": definition["platforms"][platform_name]["channels"],
                "required_assets": ordered_unique(assets),
            }
            for platform_name, assets in expected["platforms"].items()
        },
        "channels": {
            channel_name: {
                "status": definition["channels"][channel_name]["status"],
                "description": definition["channels"][channel_name]["description"],
                "required_assets": ordered_unique(assets),
            }
            for channel_name, assets in expected["channels"].items()
        },
    }
    pathlib.Path(args.output).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return 0


def linux_package_metadata_command(args: argparse.Namespace) -> int:
    results = validate_linux_package_metadata(pathlib.Path(args.deb_path), pathlib.Path(args.rpm_path))
    if args.summary_file:
        lines = [
            "## Linux package dependency metadata",
            "",
            f"Validated `{args.deb_path}` and `{args.rpm_path}` against `crates/gestura-gui/tauri.conf.json`.",
            "",
            f"- DEB depends: {', '.join(f'`{dep}`' for dep in results['expected_deb'])}",
            f"- RPM depends: {', '.join(f'`{dep}`' for dep in results['expected_rpm'])}",
        ]
        with pathlib.Path(args.summary_file).open("a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    metadata = subparsers.add_parser("metadata", help="Validate release metadata and emit GitHub outputs.")
    metadata.add_argument("--github-output", help="Path to GITHUB_OUTPUT to append key/value pairs to.")
    metadata.set_defaults(func=metadata_command)
    artifacts = subparsers.add_parser("artifacts", help="Validate the assembled release artifact set.")
    artifacts.add_argument("--tag", required=True)
    artifacts.add_argument("--dist-dir", required=True)
    artifacts.add_argument("--summary-file", help="Write a Markdown summary to the provided file.")
    artifacts.set_defaults(func=artifacts_command)
    manifest = subparsers.add_parser("manifest", help="Generate a release manifest for a validated artifact set.")
    manifest.add_argument("--tag", required=True)
    manifest.add_argument("--dist-dir", required=True)
    manifest.add_argument("--output", required=True)
    manifest.set_defaults(func=manifest_command)
    linux_metadata = subparsers.add_parser(
        "linux-package-metadata",
        help="Validate built Linux DEB/RPM dependency metadata against tauri.conf.json.",
    )
    linux_metadata.add_argument("--deb-path", required=True)
    linux_metadata.add_argument("--rpm-path", required=True)
    linux_metadata.add_argument("--summary-file", help="Append a Markdown summary to the provided file.")
    linux_metadata.set_defaults(func=linux_package_metadata_command)
    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())