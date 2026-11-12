pluginManagement {
    val flutterSdkPath =
        run {
            val properties = java.util.Properties()
            file("local.properties").inputStream().use { properties.load(it) }
            val flutterSdkPath = properties.getProperty("flutter.sdk")
            require(flutterSdkPath != null) { "flutter.sdk not set in local.properties" }
            flutterSdkPath
        }

    includeBuild("$flutterSdkPath/packages/flutter_tools/gradle")

    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

plugins {
    id("dev.flutter.flutter-plugin-loader") version "1.0.0"
    // ⚠️ **AGP 9.1.0 -> 8.13.1**（2026-11-05 修）。
    //
    // AGP 9 强制 `android.newDsl=true`，而本机 Flutter **3.38** 的
    // `dev.flutter.flutter-gradle-plugin` 仍依赖**旧 DSL 类型** ->
    // 插件应用直接失败：
    //   "> Failed to apply plugin 'dev.flutter.flutter-gradle-plugin'"
    //   "> java.lang.NullPointerException (no error message)"
    //
    // Flutter 给的两条路（flutter_tools/lib/src/android/gradle_errors.dart:704-706）：
    // 「update flutter」或「opt out of `android.newDsl`」——但 `android.newDsl`
    // 在 AGP 9 里已经**不再作为可识别的开关**（AGP 9.1.0 的 jar 里，
    // `newDsl` 只出现在 `AppExtension`/`BaseExtension` 上，语义已变；
    // 而 AGP 8.13.1 的 jar 里它还只是 `DeprecationReporter` 里的废弃项）。
    // 所以退路实际不可用，**只能升级 Flutter 或降 AGP**。
    //
    // 选降 AGP：升级 Flutter SDK 是跨项目的重动作（pub get 已提示需要
    // 3.47.9），而 8.13.1 是 8.x 的最新版，**完全支持 compileSdk 35/36**
    // （compileSdk 与 AGP 版本是两件事）。本机 Gradle 缓存里已有 8.13.1
    // 与 Kotlin 2.2.20，离线可用。
    id("com.android.application") version "8.13.1" apply false
    // Kotlin 2.2.20：与 AGP 8.13 官方搭配；2.4.0 与 AGP 8.x 的组合未被验证。
    id("org.jetbrains.kotlin.android") version "2.2.20" apply false
}

include(":app")
