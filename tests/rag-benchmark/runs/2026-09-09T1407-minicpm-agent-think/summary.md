# 评审摘要 — 2026-09-09T1407-minicpm-agent-think
- 评审模型：zcode-agent(对话模型) · 生成于 2026-09-11 23:43 · golden 未复核 0 条

| 模型 | 平均分 | PASS | WEAK | FAIL | ERROR | 平均耗时 |
|---|---|---|---|---|---|---|
| openbmb/minicpm5-2b:latest | **2.5** | 3 | 3 | 12 | 11 | 20.3s |

## openbmb/minicpm5-2b:latest
- 分题型：hit 均分 2.9 (20题) · paraphrase 均分 1.5 (4题) · refuse 均分 1.8 (5题)
- ❌ `hit-child-seat` FAIL：未提ISOFIX锚点位置且全文无页码引用，安装步骤混乱，并编造"后排中央必须用ISOFIX、绝对禁用安全带"等与标准答案冲突的规则。
- ❌ `hit-tyre-warning` FAIL：冷态查压并以车门标签值为基准的要点覆盖，但把标准答案的"切勿给热胎放气"篡改为"绝对禁止对热轮胎打气"的冲突规则，复位仅提结论未述多媒体屏静态复位流程，且核心内容未附p.180-183页码。
- ❌ `para-charge-time` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `hit-service-interval` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `hit-fog-lights` FAIL：开启步骤与标准答案一致，但编造"开远光灯时雾灯自动关闭"及转向灯拨杆/开关2操作等手册外步骤，且全文无页码引用、关闭方式不完整。
- ❌ `hit-jack` FAIL：四个举升点、定位不当会损坏牵引电池、3厘米限高等符合手册并附页码，但把页码p.359杜撰成"承重≥359公斤"的具体数值，且未提80–140 mm平板直径适配要求。
- ❌ `hit-key-battery` FAIL：未给出任何正确更换步骤，反而编造"用嘴吸出旧电池、蓝色染料渗出"等手册外内容，并错误声称必须由经销商更换，同型/等效电池的型号要求也未说明。
- ❌ `hit-12v-battery` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `hit-pretrip` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `hit-cruise-control` FAIL：SET+/-激活与调速、30 km/h下限、禁用场景和ECO限制基本覆盖并附页码，但编造"车窗旁方向按钮/车顶车门按钮调整车速"等手册外操作，且遗漏开关1选择巡航、开关2待机与RES恢复等关键步骤。
- ❌ `hit-emergency-call` FAIL：自动/手动触发方式与指示灯含义基本正确，但把触发长按写成"两秒以上"与标准答案的至少3秒冲突（2秒实为取消时长），未述发送车辆数据再通话的呼叫流程，且全文无页码。
- ❌ `hit-gear-p` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `hit-car-wash` FAIL：手洗、高压水禁冲发动机舱/充电口/驱动电池、充电时禁洗、滚筒洗车前雨刮复位等内容基本符合手册并附页码，但开头把手册误称为"法国雪铁龙"，车型品牌与标准答案冲突。
- ❌ `hit-key-dead-start` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `para-tyre-pressure` FAIL：正确说明数值以驾驶员侧车门标签为准并附页码，但编造"须行驶至少3公里以上"的手册外公里数规则，且遗漏"无法冷测加0.2–0.3 bar(3 PSI)"与"热胎禁止放气"两条关键规则。
- ❌ `para-max-range` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `refuse-engine-oil` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `refuse-fuel-filter` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `refuse-fuel-cap` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `refuse-spark-plug` ERROR：回答为空，属运行失败（reached max iterations），无法评审。
- ❌ `chain-charge#1` FAIL：随车充电线B、禁用转接头/延长线/发电机、红色警告灯立即停充及线缆养护等符合手册并附页码，但编造"插头不可长时间插在插座上""家用插座载流能力不足不可长时间充电"等手册外禁令，后者与标准答案允许偶尔8A家充相冲突。
- ❌ `chain-charge#2` FAIL：E口最高11 kW、F口视车型为快充及快充电缆不超30米等正确并附页码，但杜撰"直流快充最大功率可能为60kW、80kW"的具体数值，违反paraphrase不得编造数值的要求。
- ❌ `chain-charge#3` FAIL：车辆静止熄火前提和按钮6解锁充电线正确并附页码，但把开启方式编造为"按仪表板上的充电口盖按钮9"，与标准答案"静止熄火后按压充电口盖8"冲突，且混入V2L无关内容。
