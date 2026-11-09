use serde::{Serialize, Deserialize};


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CustomConfig{
    pub open_custom_ua: bool, //是否开启自定义UA
    pub custom_ua: String,      //自定义UA
    pub captcha_mode: usize,     //验证码模式   //0:本地打码  1：ttocr
    pub ttocr_key: String,      //ttocr key
    pub preinput_phone1: String, //预填账号1手机号
    pub preinput_phone2: String, //预填账号2手机号
    #[serde(default)]
    pub stock_check: bool,
    #[serde(default)]
    pub sale_delay_ms: u64,
    #[serde(default = "default_stock_interval_ms")]
    pub stock_interval_ms: u64,
    #[serde(default = "default_create_interval_ms")]
    pub create_interval_ms: u64,
    

}

impl CustomConfig{
    pub fn new() -> Self{
        Self{
            open_custom_ua: true,
            custom_ua: String::from("Mozilla/5.0 (Linux; Android 6.0; Nexus 5 Build/MRA58N) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Mobile Safari/537.36"),
            captcha_mode: 0,
            ttocr_key: String::new(),
            preinput_phone1: String::new(),
            preinput_phone2: String::new(),
            stock_check: false,
            sale_delay_ms: 0,
            stock_interval_ms: default_stock_interval_ms(),
            create_interval_ms: default_create_interval_ms(),
        }
    }

    pub fn validate_purchase(&self) -> Result<(), String> {
        if self.stock_interval_ms == 0 || self.create_interval_ms == 0 {
            return Err("库存查询间隔和下单间隔必须大于 0 毫秒".to_string());
        }
        if self.sale_delay_ms > i64::MAX as u64 {
            return Err("开售延迟超出支持范围".to_string());
        }
        Ok(())
    }

    pub fn sale_target_millis(&self, sale_begin: i64) -> Result<i64, String> {
        let delay = i64::try_from(self.sale_delay_ms)
            .map_err(|_| "开售延迟超出支持范围".to_string())?;
        sale_begin.checked_mul(1000).and_then(|time| time.checked_add(delay))
            .ok_or_else(|| "开售时间与延迟超出支持范围".to_string())
    }
}

fn default_stock_interval_ms() -> u64 { 2000 }
fn default_create_interval_ms() -> u64 { 400 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_keeps_values_and_defaults_new_strategy() {
        let mut legacy = serde_json::to_value(CustomConfig::new()).unwrap();
        for field in ["stock_check", "sale_delay_ms", "stock_interval_ms", "create_interval_ms"] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        legacy["custom_ua"] = "saved-agent".into();
        legacy["captcha_mode"] = 1.into();
        let loaded: CustomConfig = serde_json::from_value(legacy).unwrap();
        assert_eq!(loaded.custom_ua, "saved-agent");
        assert_eq!(loaded.captcha_mode, 1);
        assert!(!loaded.stock_check);
        assert_eq!(loaded.sale_delay_ms, 0);
        assert_eq!(loaded.stock_interval_ms, 2000);
        assert_eq!(loaded.create_interval_ms, 400);
    }

    #[test]
    fn strategy_round_trip_and_invalid_intervals() {
        let mut config = CustomConfig::new();
        config.stock_check = true;
        config.sale_delay_ms = 125;
        config.stock_interval_ms = 2300;
        config.create_interval_ms = 750;
        let restored: CustomConfig = serde_json::from_str(&serde_json::to_string(&config).unwrap()).unwrap();
        assert!(restored.stock_check);
        assert_eq!(restored.sale_target_millis(100).unwrap(), 100125);
        assert_eq!(restored.stock_interval_ms, 2300);
        assert_eq!(restored.create_interval_ms, 750);
        assert!(restored.validate_purchase().is_ok());
        config.stock_interval_ms = 0;
        assert!(config.validate_purchase().is_err());
        config.stock_interval_ms = 1;
        config.create_interval_ms = 0;
        assert!(config.validate_purchase().is_err());
        assert!(config.sale_target_millis(i64::MAX).is_err());
    }
}
