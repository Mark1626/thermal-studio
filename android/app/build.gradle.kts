import java.io.File

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "org.ossthermal.studio"
    // Current AndroidX (Compose 1.12, core 1.19) must be compiled against API 37.
    // This doesn't affect the devices we run on (minSdk) or runtime behaviour (targetSdk).
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "org.ossthermal.studio"
        minSdk = 33
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures { compose = true }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.kotlinx.coroutines.android)
    // UniFFI's generated Kotlin talks to the Rust library through JNA.
    implementation(libs.jna) { artifact { type = "aar" } }
}

// --- Rust: build libthermal_ffi.so with cargo-ndk and generate Kotlin bindings -------

val repoRoot: File = rootDir.parentFile
val cargo: String = File(System.getProperty("user.home"), ".cargo/bin/cargo")
    .takeIf { it.exists() }?.absolutePath ?: "cargo"
val jniLibsDir = layout.projectDirectory.dir("src/main/jniLibs")
val bindingsDir = layout.projectDirectory.dir("src/main/java")
val ndkDir = androidComponents.sdkComponents.ndkDirectory
// Where cargo-ndk builds the library (it then copies it into jniLibs). uniffi-bindgen reads it from here.
val rustLib: File = repoRoot.resolve("target/aarch64-linux-android/release/libthermal_ffi.so")

val cargoBuildRust = tasks.register<Exec>("cargoBuildRust") {
    group = "rust"
    description = "Builds thermal-ffi for arm64-v8a with cargo-ndk (release)."
    workingDir = repoRoot
    inputs.files(fileTree(repoRoot.resolve("crates")) { include("**/*.rs", "**/Cargo.toml", "**/*.ttf") })
    inputs.file(repoRoot.resolve("Cargo.lock"))
    outputs.dir(jniLibsDir)
    // Also an output so a `cargo clean` (which leaves jniLibs behind) makes this task run again.
    outputs.file(rustLib)
    commandLine(
        cargo, "ndk", "-t", "arm64-v8a", "--platform", "33",
        "-o", jniLibsDir.asFile.absolutePath,
        "build", "--release", "-p", "thermal-ffi",
    )
    doFirst { environment("ANDROID_NDK_HOME", ndkDir.get().asFile.absolutePath) }
}

val generateUniffiBindings = tasks.register<Exec>("generateUniffiBindings") {
    group = "rust"
    description = "Generates Kotlin bindings (package uniffi.thermal_ffi) from the built library."
    dependsOn(cargoBuildRust)
    workingDir = repoRoot
    inputs.file(rustLib)
    outputs.dir(bindingsDir.dir("uniffi"))
    commandLine(
        cargo, "run", "--quiet", "-p", "thermal-ffi", "--features", "bindgen", "--bin", "uniffi-bindgen", "--",
        "generate", "--library", rustLib.absolutePath, "--language", "kotlin", "--no-format",
        "--out-dir", bindingsDir.asFile.absolutePath,
    )
}

tasks.named("preBuild") { dependsOn(generateUniffiBindings) }
