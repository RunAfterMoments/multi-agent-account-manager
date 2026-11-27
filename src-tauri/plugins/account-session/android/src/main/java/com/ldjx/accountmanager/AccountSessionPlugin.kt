package com.ldjx.accountmanager

import android.app.Activity
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.provider.Settings
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.core.content.FileProvider
import androidx.webkit.ProfileStore
import androidx.webkit.WebViewFeature
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.net.HttpURLConnection
import java.net.URL
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlin.concurrent.thread
import org.json.JSONObject

private const val API_KEY_ALIAS = "maam-api-key"
private const val API_KEY_PREFS_NAME = "maam-api-keys"
private const val API_KEY_GCM_IV_BYTES = 12
private const val API_KEY_GCM_TAG_BITS = 128

@InvokeArg
class SubscribeEventsArgs {
    lateinit var channel: Channel
}

@InvokeArg
class OpenSessionArgs {
    var providerId: String = ""
    var providerName: String = ""
    var uuid: String = ""
    var alias: String = ""
    var autoAlias: Boolean = false
    var entryUrl: String = ""
    var allowedHosts: Array<String> = emptyArray()
    var proxyUrl: String? = null
    var timezone: String? = null
    var timezoneInitScript: String? = null
    var switchSeq: Long = 0L
    var viewInstanceId: Long = 0L
    var isColdStart: Boolean = false
}

@InvokeArg
class UpdateTitleArgs {
    var switchSeq: Long = 0L
    var title: String = ""
}

@InvokeArg
class CloseSessionArgs {
    var switchSeq: Long = 0L
}

@InvokeArg
class DeleteProfileArgs {
    var uuid: String = ""
}

@InvokeArg
class CopyProfileCookieArgs {
    var uuid: String = ""
    var url: String = ""
}

@InvokeArg
class ImportProfileCookiesArgs {
    var uuid: String = ""
    var url: String = ""
    var cookies: Array<String> = emptyArray()
}

@InvokeArg
class OpenExternalUrlArgs {
    var url: String = ""
}

@InvokeArg
class FetchTextArgs {
    var url: String = ""
    var userAgent: String = ""
    var accept: String? = null
}

@InvokeArg
class DownloadApkArgs {
    var url: String = ""
    var sha256: String? = null
    var filename: String = ""
    var userAgent: String = ""
}

@InvokeArg
class InstallApkArgs {
    var path: String = ""
}

@InvokeArg
class StoreApiKeyArgs {
    var keyRef: String = ""
    var key: String = ""
}

@InvokeArg
class LoadApiKeyArgs {
    var keyRef: String = ""
}

@InvokeArg
class DeleteApiKeyArgs {
    var keyRef: String = ""
}

@TauriPlugin
class AccountSessionPlugin(private val activity: Activity) : Plugin(activity) {
    @Volatile
    private var lastDownloadedUrl: String? = null

    @Volatile
    private var lastDownloadedPath: String? = null

    // API Key 读写互斥：并发 store 时两个线程同时进 getOrCreateApiKey()
    // 会互相覆盖 Keystore 里的 maam-api-key 主密钥，导致先写的密文
    // 永久解不开；load 的取密钥+解密也同步，避免与 store 的密钥轮换竞态。
    private val apiKeyLock = Any()

    @Command
    fun subscribeEvents(invoke: Invoke) {
        val args = invoke.parseArgs(SubscribeEventsArgs::class.java)
        AccountSessionCoordinator.setEventChannel(args.channel)
        invoke.resolve(JSObject())
    }

    @Command
    fun checkCapabilities(invoke: Invoke) {
        activity.runOnUiThread {
            try {
                val supported = WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)
                val profiles = JSArray()
                if (supported) {
                    for (name in ProfileStore.getInstance().allProfileNames) {
                        profiles.put(name)
                    }
                }
                val ret = JSObject().apply {
                    put("multiProfileSupported", supported)
                    put("profileNames", profiles)
                }
                invoke.resolve(ret)
            } catch (t: Throwable) {
                invoke.reject(t.message ?: "检测 Android WebView 能力异常")
            }
        }
    }

    @Command
    fun openAccountSession(invoke: Invoke) {
        val args = invoke.parseArgs(OpenSessionArgs::class.java)
        activity.runOnUiThread {
            // M-03 fail-closed：proxyUrl 自 Rust 移动端桥接传入（与 iOS 共用，
            // iOS 侧由 AccountWebViewController 经 WKWebView 配置真正应用），
            // 但 Android 侧未接 ProxyController，此处若直接丢弃，账号会
            // 在不走代理的情况下打开。非空即拒绝，不静默降级；这是唯一的
            // fail-closed 点——Rust 侧不拦，以免误伤支持代理的 iOS。
            if (!args.proxyUrl.isNullOrBlank()) {
                invoke.reject("账号代理暂不支持 Android，请先移除该账号的代理设置，或在桌面端打开")
                return@runOnUiThread
            }
            if (!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
                invoke.reject("系统 WebView 不支持 MULTI_PROFILE")
                return@runOnUiThread
            }
            if (args.isColdStart) {
                val allNames = ProfileStore.getInstance().allProfileNames
                if (!allNames.contains(args.uuid)) {
                    invoke.reject("已提交账号的 Profile 不存在")
                    return@runOnUiThread
                }
            }

            val spec = AndroidAccountSessionSpec(
                providerId = args.providerId,
                providerName = args.providerName,
                uuid = args.uuid,
                alias = args.alias,
                autoAlias = args.autoAlias,
                entryUrl = args.entryUrl,
                allowedHosts = args.allowedHosts.toList(),
                timezone = args.timezone,
                timezoneInitScript = args.timezoneInitScript,
                switchSeq = args.switchSeq,
                viewInstanceId = args.viewInstanceId,
                isColdStart = args.isColdStart,
            )

            val existing = AccountSessionCoordinator.beginSession(spec)
            if (!AccountSessionCoordinator.isCurrent(spec.switchSeq, spec.viewInstanceId)) {
                invoke.reject("切换请求已过期")
                return@runOnUiThread
            }
            if (existing != null && !existing.isFinishing && !existing.isDestroyed) {
                existing.bindSessionSpec(spec)
            } else {
                val intent = Intent(activity, AccountWebActivity::class.java).apply {
                    addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
                }
                activity.startActivity(intent)
            }
            invoke.resolve(JSObject())
        }
    }

    @Command
    fun updateAccountTitle(invoke: Invoke) {
        val args = invoke.parseArgs(UpdateTitleArgs::class.java)
        activity.runOnUiThread {
            AccountSessionCoordinator.markCommittedAndReveal(args.switchSeq, args.title)
            invoke.resolve(JSObject())
        }
    }

    @Command
    fun closeActiveSession(invoke: Invoke) {
        val args = invoke.parseArgs(CloseSessionArgs::class.java)
        activity.runOnUiThread {
            val previous = AccountSessionCoordinator.currentActiveSpec()
            val act = AccountSessionCoordinator.invalidateAndClose(args.switchSeq)
            act?.closeAndFinishFromManager(keepPage = previous != null && args.switchSeq > previous.switchSeq)
            invoke.resolve(JSObject())
        }
    }

    @Command
    fun deleteProfile(invoke: Invoke) {
        val args = invoke.parseArgs(DeleteProfileArgs::class.java)
        activity.runOnUiThread {
            try {
                if (!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
                    invoke.resolve(JSObject())
                    return@runOnUiThread
                }
                if (AccountSessionCoordinator.isUuidActive(args.uuid)) {
                    invoke.reject("目标 Profile 视图尚未销毁")
                    return@runOnUiThread
                }
                RecentAccountPages.discardProfile(args.uuid)
                val store = ProfileStore.getInstance()
                if (store.allProfileNames.contains(args.uuid)) {
                    val deleted = store.deleteProfile(args.uuid)
                    if (!deleted) {
                        invoke.reject("系统 Profile 正在被占用，待下次启动重试")
                        return@runOnUiThread
                    }
                }
                invoke.resolve(JSObject())
            } catch (t: Throwable) {
                invoke.reject(t.message ?: "删除 Android Profile 失败")
            }
        }
    }

    @Command
    fun copyProfileCookie(invoke: Invoke) {
        val args = invoke.parseArgs(CopyProfileCookieArgs::class.java)
        activity.runOnUiThread {
            try {
                if (!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
                    invoke.reject("系统 WebView 不支持 MULTI_PROFILE")
                    return@runOnUiThread
                }
                val cookies = ProfileStore.getInstance().getProfile(args.uuid)
                    ?.cookieManager?.getCookie(args.url)
                if (cookies.isNullOrEmpty()) {
                    invoke.reject("该账号暂无可复制的 Cookie，请先登录")
                    return@runOnUiThread
                }
                val clipboard = activity.getSystemService(ClipboardManager::class.java)
                if (clipboard == null) {
                    invoke.reject("无法写入系统剪贴板，请重试")
                    return@runOnUiThread
                }
                val clip = ClipData.newPlainText("Cookie", cookies)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    clip.description.extras = PersistableBundle().apply {
                        putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
                    }
                }
                clipboard.setPrimaryClip(clip)
                invoke.resolve(JSObject())
                // 60 秒后自动清理剪贴板：只在内容仍是本次复制的 Cookie 时才清。
                Handler(Looper.getMainLooper()).postDelayed({
                    try {
                        val currentText = clipboard.primaryClip?.getItemAt(0)?.text?.toString()
                        if (currentText == cookies) {
                            clipboard.setPrimaryClip(ClipData.newPlainText("", ""))
                        }
                    } catch (_: Exception) {
                    }
                }, 60_000)
            } catch (error: Exception) {
                invoke.reject("复制 Cookie 失败，请稍后重试")
            }
        }
    }

    @Command
    fun importProfileCookies(invoke: Invoke) {
        val args = invoke.parseArgs(ImportProfileCookiesArgs::class.java)
        activity.runOnUiThread {
            try {
                if (!WebViewFeature.isFeatureSupported(WebViewFeature.MULTI_PROFILE)) {
                    invoke.reject("系统 WebView 不支持 MULTI_PROFILE")
                    return@runOnUiThread
                }
                // 新账号从未打开过时 profile 还不存在（profile 由打开会话
                // 的 WebViewCompat.setProfile 创建），getProfile 会返回 null
                // 导致导入直接失败；用 getOrCreateProfile（webkit 1.18.0
                // 已提供）按需创建。
                val cookieManager = ProfileStore.getInstance().getOrCreateProfile(args.uuid)?.cookieManager
                if (cookieManager == null) {
                    invoke.reject("无法访问该账号的隔离 Cookie 存储")
                    return@runOnUiThread
                }
                cookieManager.setAcceptCookie(true)
                for (header in args.cookies) {
                    if (header.isNotBlank()) {
                        cookieManager.setCookie(args.url, header)
                    }
                }
                cookieManager.flush()
                invoke.resolve(JSObject())
            } catch (error: Exception) {
                invoke.reject("导入 Cookie 失败，请检查格式后重试")
            }
        }
    }

    @Command
    fun openExternalUrl(invoke: Invoke) {
        val args = invoke.parseArgs(OpenExternalUrlArgs::class.java)
        activity.runOnUiThread {
            try {
                val uri = Uri.parse(args.url)
                val scheme = uri.scheme?.lowercase() ?: ""
                if (scheme != "http" && scheme != "https") {
                    invoke.reject("仅允许打开 http/https 外部链接")
                    return@runOnUiThread
                }
                val intent = Intent(Intent.ACTION_VIEW, uri).apply {
                    addCategory(Intent.CATEGORY_BROWSABLE)
                    addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                }
                activity.startActivity(intent)
                invoke.resolve(JSObject())
            } catch (t: Throwable) {
                invoke.reject(t.message ?: "无法调用系统浏览器打开链接")
            }
        }
    }

    @Command
    fun fetchText(invoke: Invoke) {
        val args = invoke.parseArgs(FetchTextArgs::class.java)
        thread(name = "maam-update-fetch") {
            var conn: HttpURLConnection? = null
            try {
                conn = openHttpConnection(args.url, args.userAgent, args.accept, 20_000)
                val status = conn.responseCode
                val stream = if (status in 200..299) conn.inputStream else conn.errorStream
                val body = if (stream != null) {
                    stream.use { input ->
                        val out = ByteArrayOutputStream()
                        val buf = ByteArray(8192)
                        var totalRead = 0
                        while (true) {
                            val n = input.read(buf)
                            if (n <= 0) break
                            totalRead += n
                            if (totalRead > 2 * 1024 * 1024) {
                                throw IllegalStateException("响应内容过大")
                            }
                            out.write(buf, 0, n)
                        }
                        out.toString(Charsets.UTF_8.name())
                    }
                } else {
                    ""
                }
                val ret = JSObject().apply {
                    put("status", status)
                    put("body", body)
                }
                invoke.resolve(ret)
            } catch (t: Throwable) {
                invoke.reject(t.message ?: "网络请求失败")
            } finally {
                conn?.disconnect()
            }
        }
    }

    @Command
    fun downloadApk(invoke: Invoke) {
        val args = invoke.parseArgs(DownloadApkArgs::class.java)
        thread(name = "maam-update-apk-dl") {
            var conn: HttpURLConnection? = null
            val rawName = args.filename.substringAfterLast('/').substringAfterLast('\\').trim()
            val safeName = rawName
                .replace(Regex("""[^A-Za-z0-9._-]"""), "_")
                .trimStart('.')
                .ifEmpty { "update.apk" }
                .let { if (it.endsWith(".apk", ignoreCase = true)) it else "$it.apk" }
            val updatesDir = File(activity.cacheDir, "updates").apply { mkdirs() }
            val targetFile = File(updatesDir, safeName)
            val partFile = File(updatesDir, "$safeName.part")
            val expectedSha = args.sha256?.trim()?.lowercase()?.takeIf { it.isNotEmpty() }

            // SHA-256 必填（P0）：缺失或格式非法时拒绝安装并删除已下载文件，
            // 不复用旧文件、不调起安装器。Rust 侧已在调用前强制要求，这里是
            // 纵深防御——绕开 Rust 命令直接调插件时也不能跳过校验。
            if (expectedSha == null || !Regex("^[a-f0-9]{64}$").matches(expectedSha)) {
                if (targetFile.exists()) targetFile.delete()
                if (partFile.exists()) partFile.delete()
                val errMsg = if (expectedSha == null) "缺少更新包 SHA-256，拒绝安装" else "SHA-256 格式无效，拒绝安装"
                AccountSessionCoordinator.emitUpdateProgress("error", 0L, null, null, errMsg)
                invoke.reject(errMsg)
                return@thread
            }

            try {
                // 复用分支一律重算摘要，不再凭 URL 信任旧文件。
                val canReuse = targetFile.exists() && targetFile.length() > 0L &&
                    computeSha256Hex(targetFile) == expectedSha

                if (!canReuse) {
                    AccountSessionCoordinator.emitUpdateProgress(
                        stage = "downloading",
                        downloaded = 0L,
                        total = null,
                        percent = 0,
                        message = "正在连接下载服务器...",
                    )
                    conn = openHttpConnection(args.url, args.userAgent, null, 60_000)
                    val status = conn.responseCode
                    if (status !in 200..299) {
                        throw IllegalStateException("下载更新包失败（HTTP $status）")
                    }
                    val totalBytes = conn.contentLengthLong.takeIf { it > 0L }
                    // 下载上限 512 MiB（与桌面 updater.rs 的 MAX_UPDATE_DOWNLOAD_BYTES
                    // 一致）：服务端谎报/不报 Content-Length 时，流式循环里按已下载量硬停。
                    val maxApkBytes = 512L * 1024L * 1024L
                    if (totalBytes != null && totalBytes > maxApkBytes) {
                        throw IllegalStateException("更新包超过 512 MiB 上限，拒绝下载")
                    }
                    val digest = MessageDigest.getInstance("SHA-256")
                    var downloaded = 0L
                    var lastEmitTime = 0L
                    var lastEmitPercent = -1

                    if (partFile.exists()) {
                        partFile.delete()
                    }
                    conn.inputStream.use { input ->
                        FileOutputStream(partFile).use { output ->
                            val buf = ByteArray(16 * 1024)
                            while (true) {
                                val n = input.read(buf)
                                if (n <= 0) break
                                output.write(buf, 0, n)
                                digest.update(buf, 0, n)
                                downloaded += n
                                if (downloaded > maxApkBytes) {
                                    throw IllegalStateException("更新包超过 512 MiB 上限，拒绝下载")
                                }
                                val pct = totalBytes?.let { ((downloaded * 100L) / it).toInt().coerceIn(0, 100) }
                                val now = System.currentTimeMillis()
                                if (now - lastEmitTime >= 150L || (pct != null && pct != lastEmitPercent)) {
                                    lastEmitTime = now
                                    if (pct != null) lastEmitPercent = pct
                                    AccountSessionCoordinator.emitUpdateProgress(
                                        stage = "downloading",
                                        downloaded = downloaded,
                                        total = totalBytes,
                                        percent = pct,
                                        message = "正在下载安装包...",
                                    )
                                }
                            }
                            output.fd.sync()
                        }
                    }

                    AccountSessionCoordinator.emitUpdateProgress(
                        stage = "verifying",
                        downloaded = downloaded,
                        total = totalBytes,
                        percent = 100,
                        message = "正在校验安装包完整性...",
                    )
                    val actualSha = digest.digest().joinToString("") { "%02x".format(it) }
                    if (actualSha != expectedSha) {
                        partFile.delete()
                        throw IllegalStateException("安装包 SHA-256 校验失败，请重试")
                    }

                    if (targetFile.exists()) {
                        targetFile.delete()
                    }
                    if (!partFile.renameTo(targetFile)) {
                        FileInputStream(partFile).use { input ->
                            FileOutputStream(targetFile).use { output ->
                                input.copyTo(output)
                                output.fd.sync()
                            }
                        }
                        partFile.delete()
                    }

                    updatesDir.listFiles()?.forEach { f ->
                        if (f.name != targetFile.name) {
                            f.delete()
                        }
                    }
                    lastDownloadedUrl = args.url
                    lastDownloadedPath = targetFile.absolutePath
                }

                val ret = JSObject().apply {
                    put("status", "downloaded")
                    put("message", "安装包已下载完成")
                    put("path", targetFile.absolutePath)
                }
                invoke.resolve(ret)
            } catch (t: Throwable) {
                if (partFile.exists()) {
                    partFile.delete()
                }
                val err = t.message ?: "下载更新包失败"
                AccountSessionCoordinator.emitUpdateProgress("error", 0L, null, null, err)
                invoke.reject(err)
            } finally {
                conn?.disconnect()
            }
        }
    }

    @Command
    fun installApk(invoke: Invoke) {
        val args = invoke.parseArgs(InstallApkArgs::class.java)
        thread(name = "maam-update-apk-install") {
            val apkFile = File(args.path)
            // 必须位于应用缓存 updates 目录下、是 .apk 文件、真实存在。
            val updatesDir = File(activity.cacheDir, "updates")
            val canonicalUpdates = updatesDir.canonicalFile
            val canonicalApk = apkFile.canonicalFile
            if (!canonicalApk.path.startsWith(canonicalUpdates.path + File.separator)
                || !canonicalApk.name.endsWith(".apk", ignoreCase = true)
                || !canonicalApk.isFile
            ) {
                invoke.reject("安装包路径不在应用缓存的 updates 目录下，拒绝安装")
                return@thread
            }
            val targetFile = canonicalApk

            activity.runOnUiThread {
                try {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
                        val permIntent = Intent(
                            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                            Uri.parse("package:${activity.packageName}"),
                        ).apply {
                            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                        }
                        activity.startActivity(permIntent)
                        val msg = "请先开启“允许安装未知来源应用”，开启后返回再次点击安装"
                        AccountSessionCoordinator.emitUpdateProgress(
                            stage = "needs_permission",
                            downloaded = targetFile.length(),
                            total = targetFile.length(),
                            percent = 100,
                            message = msg,
                        )
                        val ret = JSObject().apply {
                            put("status", "needs_permission")
                            put("message", msg)
                        }
                        invoke.resolve(ret)
                        return@runOnUiThread
                    }

                    val apkUri = FileProvider.getUriForFile(
                        activity,
                        "${activity.packageName}.fileprovider",
                        targetFile,
                    )
                    val installIntent = Intent(Intent.ACTION_VIEW).apply {
                        setDataAndType(apkUri, "application/vnd.android.package-archive")
                        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_GRANT_READ_URI_PERMISSION)
                    }
                    activity.startActivity(installIntent)
                    val msg = "已调起系统安装器，请按提示完成覆盖更新"
                    AccountSessionCoordinator.emitUpdateProgress(
                        stage = "ready",
                        downloaded = targetFile.length(),
                        total = targetFile.length(),
                        percent = 100,
                        message = msg,
                    )
                    val ret = JSObject().apply {
                        put("status", "installer_launched")
                        put("message", msg)
                    }
                    invoke.resolve(ret)
                } catch (t: Throwable) {
                    val err = t.message ?: "调起系统安装器失败"
                    AccountSessionCoordinator.emitUpdateProgress("error", 0L, null, null, err)
                    invoke.reject(err)
                }
            }
        }
    }

    @Command
    fun storeApiKey(invoke: Invoke) {
        val args = invoke.parseArgs(StoreApiKeyArgs::class.java)
        thread(name = "maam-api-key") {
            try {
                if (args.keyRef.isBlank()) {
                    invoke.reject("API Key 标识不能为空")
                    return@thread
                }
                synchronized(apiKeyLock) {
                    val secretKey = getOrCreateApiKey()
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                    cipher.init(Cipher.ENCRYPT_MODE, secretKey)
                    // AAD 绑定 keyRef：防密文跨条目替换（load 用同一 keyRef 校验）。
                    cipher.updateAAD(args.keyRef.toByteArray(Charsets.UTF_8))
                    val iv = cipher.iv ?: throw IllegalStateException("无法生成加密 IV")
                    val ciphertext = cipher.doFinal(args.key.toByteArray(Charsets.UTF_8))
                    val stored = Base64.encodeToString(iv + ciphertext, Base64.NO_WRAP)
                    // 同步 commit 并校验：异步 apply() 在 resolve 后崩溃可能丢 Key。
                    if (!apiKeyPrefs().edit().putString(args.keyRef, stored).commit()) {
                        throw IllegalStateException("写入 API Key 失败")
                    }
                }
                invoke.resolve(JSObject())
            } catch (t: Throwable) {
                invoke.reject("保存 API Key 失败")
            }
        }
    }

    @Command
    fun loadApiKey(invoke: Invoke) {
        val args = invoke.parseArgs(LoadApiKeyArgs::class.java)
        thread(name = "maam-api-key") {
            try {
                if (args.keyRef.isBlank()) {
                    invoke.reject("API Key 标识不能为空")
                    return@thread
                }
                val stored = apiKeyPrefs().getString(args.keyRef, null)
                if (stored == null) {
                    val ret = JSObject().apply {
                        put("value", JSONObject.NULL)
                    }
                    invoke.resolve(ret)
                    return@thread
                }
                val combined = Base64.decode(stored, Base64.NO_WRAP)
                if (combined.size <= API_KEY_GCM_IV_BYTES) {
                    invoke.reject("已保存的 API Key 数据损坏，无法解密")
                    return@thread
                }
                // 注意：这里只取密钥、不新建。条目存在但密钥丢失时解密注定失败，
                // 按设计直接 reject，不静默删数据、不重建密钥。
                val plaintext = synchronized(apiKeyLock) {
                    val secretKey = getApiKey()
                        ?: throw IllegalStateException("Android Keystore 密钥不可用")
                    val iv = combined.copyOfRange(0, API_KEY_GCM_IV_BYTES)
                    val ciphertext = combined.copyOfRange(API_KEY_GCM_IV_BYTES, combined.size)
                    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                    cipher.init(Cipher.DECRYPT_MODE, secretKey, GCMParameterSpec(API_KEY_GCM_TAG_BITS, iv))
                    // AAD 与 store 用同一 keyRef：密文被换到别的条目时解密失败。
                    cipher.updateAAD(args.keyRef.toByteArray(Charsets.UTF_8))
                    cipher.doFinal(ciphertext)
                }
                val ret = JSObject().apply {
                    put("value", String(plaintext, Charsets.UTF_8))
                }
                invoke.resolve(ret)
            } catch (t: Throwable) {
                invoke.reject("读取 API Key 失败")
            }
        }
    }

    @Command
    fun deleteApiKey(invoke: Invoke) {
        val args = invoke.parseArgs(DeleteApiKeyArgs::class.java)
        thread(name = "maam-api-key") {
            try {
                if (args.keyRef.isBlank()) {
                    invoke.reject("API Key 标识不能为空")
                    return@thread
                }
                // remove 不存在的 key 也是成功（SharedPreferences.Editor.remove 无此 key 时静默无操作）。
                // 同步 commit 并校验，与 store 对齐。
                synchronized(apiKeyLock) {
                    if (!apiKeyPrefs().edit().remove(args.keyRef).commit()) {
                        throw IllegalStateException("删除 API Key 失败")
                    }
                }
                invoke.resolve(JSObject())
            } catch (t: Throwable) {
                invoke.reject("删除 API Key 失败")
            }
        }
    }

    private fun openHttpConnection(
        rawUrl: String,
        userAgent: String,
        accept: String?,
        readTimeoutMs: Int,
    ): HttpURLConnection {
        var currentUrl = URL(rawUrl)
        if (!currentUrl.protocol.equals("https", ignoreCase = true)) {
            throw IllegalArgumentException("仅允许 HTTPS 更新地址")
        }
        // 重定向白名单（与 Rust updater.rs 的 is_allowed_redirect_target 一致）：
        // 初始地址与每一跳都必须是这四个精确主机，不含子域名。
        val allowedHosts = setOf(
            "github.com",
            "api.github.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com"
        )
        if (!allowedHosts.contains(currentUrl.host.lowercase())) {
            throw IllegalStateException("重定向目标主机不在白名单内")
        }
        for (hop in 0 until 10) {
            val conn = (currentUrl.openConnection() as HttpURLConnection).apply {
                connectTimeout = 15_000
                readTimeout = readTimeoutMs
                instanceFollowRedirects = false
                if (userAgent.isNotEmpty()) {
                    setRequestProperty("User-Agent", userAgent)
                }
                if (!accept.isNullOrEmpty()) {
                    setRequestProperty("Accept", accept)
                }
            }
            val code = conn.responseCode
            if (code == HttpURLConnection.HTTP_MOVED_PERM ||
                code == HttpURLConnection.HTTP_MOVED_TEMP ||
                code == HttpURLConnection.HTTP_SEE_OTHER ||
                code == 307 ||
                code == 308
            ) {
                val location = conn.getHeaderField("Location")
                conn.disconnect()
                if (location.isNullOrEmpty()) {
                    throw IllegalStateException("重定向缺少 Location 头")
                }
                val nextUrl = URL(currentUrl, location)
                if (!nextUrl.protocol.equals("https", ignoreCase = true)) {
                    throw IllegalStateException("重定向到非 HTTPS 地址已被拒绝")
                }
                if (!allowedHosts.contains(nextUrl.host.lowercase())) {
                    throw IllegalStateException("重定向目标主机不在白名单内")
                }
                currentUrl = nextUrl
                continue
            }
            return conn
        }
        throw IllegalStateException("网络重定向次数过多")
    }

    private fun computeSha256Hex(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        FileInputStream(file).use { input ->
            val buf = ByteArray(16 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n <= 0) break
                digest.update(buf, 0, n)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun apiKeyPrefs(): SharedPreferences {
        return activity.getSharedPreferences(API_KEY_PREFS_NAME, Context.MODE_PRIVATE)
    }

    private fun getApiKey(): SecretKey? {
        val keyStore = KeyStore.getInstance("AndroidKeyStore")
        keyStore.load(null)
        return keyStore.getKey(API_KEY_ALIAS, null) as? SecretKey
    }

    private fun getOrCreateApiKey(): SecretKey {
        getApiKey()?.let { return it }
        val spec = KeyGenParameterSpec.Builder(
            API_KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        ).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build()
        val keyGenerator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        keyGenerator.init(spec)
        return keyGenerator.generateKey()
    }
}
