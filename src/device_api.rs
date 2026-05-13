// 设备管理API
use crate::api::{ApiResponse, ApiState, jwt_user_id_from_headers, DeviceInfo};
use crate::database::CreateDeviceRequest;
use axum::{
    extract::{Extension, Path},
    http::{HeaderMap, StatusCode},
    response::Json,
};use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AddDeviceRequest {
    pub device_id: String,
    pub device_name: Option<String>,
}

pub async fn add_device(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Json(request): Json<AddDeviceRequest>,
) -> Result<Json<ApiResponse<DeviceInfo>>, StatusCode> {
    let user_id = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;

    let create_request = CreateDeviceRequest {
        user_id,
        device_id: request.device_id.clone(),
        device_name: request.device_name.clone(),
    };
    
    match state.db.add_device_to_user(&create_request).await {
        Ok(_device_id) => {
            // 获取刚创建的设备信息
            match state.db.get_user_devices(user_id).await {
                Ok(devices) => {
                    if let Some(device) = devices.iter().find(|d| d.device_id == request.device_id) {
                        Ok(Json(ApiResponse::success(device.clone().into())))
                    } else {
                        Ok(Json(ApiResponse::error("创建设备后查询失败".to_string())))
                    }
                }
                Err(_) => {
                    Ok(Json(ApiResponse::error("查询设备信息失败".to_string())))
                }
            }
        }
        Err(e) => {
            Ok(Json(ApiResponse::error(format!("添加设备失败: {}", e))))
        }
    }
}

pub async fn remove_device_by_id(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    let user_id = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;

    match state.db.remove_device_from_user(user_id, &device_id).await {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::<()>::error(format!("删除设备失败: {}", e)))),
    }
}
