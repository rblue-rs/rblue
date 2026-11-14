#!/usr/bin/env python3
"""Migrate a disposable plugin example copy to AGP 9 built-in Kotlin."""

from pathlib import Path
import sys

root = Path(sys.argv[1])
android = root / "example" / "android"

settings = android / "settings.gradle.kts"
text = settings.read_text()
text = text.replace(
    'id("com.android.application") version "8.11.1" apply false',
    'id("com.android.application") version "9.0.1" apply false',
)
text = text.replace(
    '    id("org.jetbrains.kotlin.android") version "2.2.20" apply false\n',
    "",
)
settings.write_text(text)

app = android / "app" / "build.gradle.kts"
text = app.read_text()
text = text.replace('    id("kotlin-android")\n', "")
text = text.replace(
    "    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.\n",
    "",
)
start = text.find("    kotlinOptions {\n")
if start >= 0:
    end = text.find("    }\n", start) + len("    }\n")
    text = text[:start] + text[end:]
app.write_text(text)

properties = android / "gradle.properties"
lines = [
    line
    for line in properties.read_text().splitlines()
    if not line.startswith(
        (
            "android.builtInKotlin=",
            "android.newDsl=",
            "# This builtInKotlin",
            "# This newDsl",
        )
    )
]
lines.append("android.newDsl=false")
properties.write_text("\n".join(lines) + "\n")

wrapper = android / "gradle" / "wrapper" / "gradle-wrapper.properties"
text = wrapper.read_text()
text = text.replace("gradle-8.14-all.zip", "gradle-9.1.0-all.zip")
text = text.replace("gradle-8.14-bin.zip", "gradle-9.1.0-bin.zip")
wrapper.write_text(text)
