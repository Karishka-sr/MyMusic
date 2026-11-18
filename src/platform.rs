//! Работа с Android: разрешение на чтение музыки. Через JNI вызываем методы Activity.
//! На других системах (сборка для проверки на ПК) — заглушки.

#[cfg(target_os = "android")]
mod imp {
    use jni::{
        objects::{JObject, JString, JValue},
        JNIEnv, JavaVM,
    };

    /// Выполняет `f` с доступом к Java: окружение + текущая Activity.
    fn with_activity<R>(
        f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>,
    ) -> Option<R> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
        match f(&mut env, &activity) {
            Ok(value) => Some(value),
            Err(_) => {
                // после ошибки в Java остаётся «висящее» исключение — убираем
                let _ = env.exception_clear();
                None
            }
        }
    }

    fn sdk_int() -> i32 {
        with_activity(|env, _| {
            env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()
        })
        .unwrap_or(0)
    }

    /// Android 13+ — отдельное право на аудио, раньше — общий доступ к памяти.
    fn permission_name() -> &'static str {
        if sdk_int() >= 33 {
            "android.permission.READ_MEDIA_AUDIO"
        } else {
            "android.permission.READ_EXTERNAL_STORAGE"
        }
    }

    pub fn has_audio_permission() -> bool {
        let name = permission_name();
        with_activity(|env, activity| {
            let perm = env.new_string(name)?;
            let result = env
                .call_method(
                    activity,
                    "checkSelfPermission",
                    "(Ljava/lang/String;)I",
                    &[JValue::Object(&perm)],
                )?
                .i()?;
            Ok(result == 0) // 0 = PERMISSION_GRANTED
        })
        .unwrap_or(false)
    }

    /// «Доступ ко всем файлам» (Android 11+). Без него нельзя удалять чужие файлы.
    pub fn has_all_files_access() -> bool {
        if sdk_int() < 30 {
            return true;
        }
        with_activity(|env, _| {
            env.call_static_method("android/os/Environment", "isExternalStorageManager", "()Z", &[])?
                .z()
        })
        .unwrap_or(false)
    }

    /// Открывает системный экран, где нужно включить переключатель «Доступ ко всем файлам».
    pub fn request_all_files_access() {
        with_activity(|env, activity| {
            let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&action)],
            )?;
            let pkg_obj = env
                .call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?
                .l()?;
            let pkg: String = env.get_string(&JString::from(pkg_obj))?.into();
            let uri_text = env.new_string(format!("package:{pkg}"))?;
            let uri = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&uri_text)],
                )?
                .l()?;
            env.call_method(
                &intent,
                "setData",
                "(Landroid/net/Uri;)Landroid/content/Intent;",
                &[JValue::Object(&uri)],
            )?;
            env.call_method(
                activity,
                "startActivity",
                "(Landroid/content/Intent;)V",
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        });
    }

    /// Сообщает системной медиатеке, что файл удалён, чтобы он пропал и из других приложений.
    pub fn notify_file_removed(path: &std::path::Path) {
        let path = path.to_string_lossy().into_owned();
        with_activity(|env, activity| {
            let array = env.new_object_array(1, "java/lang/String", JObject::null())?;
            let p = env.new_string(path)?;
            env.set_object_array_element(&array, 0, p)?;
            let null = JObject::null();
            env.call_static_method(
                "android/media/MediaScannerConnection",
                "scanFile",
                "(Landroid/content/Context;[Ljava/lang/String;[Ljava/lang/String;Landroid/media/MediaScannerConnection$OnScanCompletedListener;)V",
                &[
                    JValue::Object(activity),
                    JValue::Object(&array),
                    JValue::Object(&null),
                    JValue::Object(&null),
                ],
            )?;
            Ok(())
        });
    }

    pub fn request_audio_permission() {
        let name = permission_name();
        with_activity(|env, activity| {
            let array = env.new_object_array(1, "java/lang/String", JObject::null())?;
            let perm = env.new_string(name)?;
            env.set_object_array_element(&array, 0, perm)?;
            env.call_method(
                activity,
                "requestPermissions",
                "([Ljava/lang/String;I)V",
                &[JValue::Object(&array), JValue::Int(7)],
            )?;
            Ok(())
        });
    }
}

#[cfg(not(target_os = "android"))]
mod imp {
    pub fn has_audio_permission() -> bool {
        true
    }
    pub fn request_audio_permission() {}
    pub fn has_all_files_access() -> bool {
        true
    }
    pub fn request_all_files_access() {}
    pub fn notify_file_removed(_path: &std::path::Path) {}
}

pub use imp::*;
