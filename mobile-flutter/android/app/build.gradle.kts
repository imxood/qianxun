plugins {
    id("com.android.application")
    // ⚠️ **必须应用 Kotlin Android 插件**（AGP 8.x 的形态）。
    //
    // 2026-11-05 曾把这里改成「不应用任何 Kotlin 插件 + 顶层 compilerOptions」，
    // 那是 **AGP 9** 的要求（AGP 9 内置 Kotlin，应用插件反而失败）。但 AGP 9
    // 与本机 Flutter 3.38 的 Gradle 插件不兼容（见 settings.gradle.kts 的注释），
    // 已改回 AGP 8.13.1 —— 于是这里必须把 Kotlin 插件应用回来。
    id("org.jetbrains.kotlin.android")
    // The Flutter Gradle Plugin must be applied after the Android and Kotlin Gradle plugins.
    id("dev.flutter.flutter-gradle-plugin")
}

android {
    namespace = "com.qianxun.qianxun_mobile"
    // ⚠️ 显式钉住 SDK 版本（2026-11-05）。
    //
    // **compileSdk 必须是 36**：插件 `webview_flutter_android` 的源码是按
    // Android SDK 36 编译的，AGP 会强制校验——
    //   "webview_flutter_android compiles against Android SDK 36 ...
    //    Fix this issue by compiling against the highest Android SDK version"
    // 用 35 会直接构建失败。（本机 platforms 有 33..36，build-tools 35/36 都在。）
    //
    // 为什么不用 `flutter.compileSdkVersion`：那个值跟着 Flutter SDK 走
    // （本机 Flutter 3.38 对应的默认值远低于 36），新 SDK 会白装。
    //
    // ⚠️ compileSdk 与 AGP 版本是两件事：AGP 8.13.1 同样支持 compileSdk 36。
    compileSdk = 36
    // NDK 版本必须与已装 NDK 一致，否则报「NDK not found」。
    ndkVersion = "28.2.13676358"

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    defaultConfig {
        // TODO: Specify your own unique Application ID (https://developer.android.com/studio/build/application-id.html).
        applicationId = "com.qianxun.qianxun_mobile"
        // minSdk 26（Android 8.0）：覆盖绝大多数在用机型，同时满足
        // file_picker / webview_flutter_android 的最低要求。
        // ⚠️ 不用 `flutter.minSdkVersion`（跟着 Flutter SDK 走，会漂）。
        minSdk = 26
        // targetSdk 36 = Android 16：与 compileSdk 对齐，跟随手机系统版本
        // （手机正是 Android 16），用上最新行为变更。
        targetSdk = 36
        // Uses the version code from pubspec.yaml. When using split APKs, 1000 * ABI_VERSION
        // is added automatically by Flutter. (https://developer.android.com/studio/build/configure-APK-splits#configure-APK-versions)
        // You can force using the value of versionCode by specifying the `-P force-version-code-ignoring-abi=true`
        // flag during build.
        versionCode = flutter.versionCode
        versionName = flutter.versionName
    }

    buildTypes {
        release {
            // TODO: Add your own signing config for the release build.
            // Signing with the debug keys for now, so `flutter run --release` works.
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

// Kotlin 编译目标：项目级 `kotlin { }` 块（由 Kotlin Android 插件提供
// `KotlinAndroidProjectExtension`）。
//
// ⚠️ 别写成顶层 `compilerOptions { }`——那是 `KotlinTopLevelExtension`
// （纯 JVM 插件）的形态，Android 模块用不了。
kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

flutter {
    source = "../.."
}
