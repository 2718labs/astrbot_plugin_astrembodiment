# A04 — 16,384 节点神经连续体

## 目标

建立一颗固定规模、动态连接、稀疏装配的 Persona 级大脑。情绪由集体场涌现，不由单个标量节点表示。

## 节点布局

| 区域 | 节点数 |
|---|---:|
| Interoception / Allostasis | 2,048 |
| Affective Valuation | 2,048 |
| Salience | 1,024 |
| Epistemic / Fallibility | 2,048 |
| Social / Boundary | 2,048 |
| Temper / Inhibitory Control | 1,024 |
| World Model / Imagination | 4,096 |
| Global Workspace | 1,024 |
| Action / Expression | 1,024 |
| 总计 | 16,384 |

其中 2,048 个槽位可作为低活跃储备，由区域配置决定可招募范围。

## 每节点自由度

\[
h_i=(v_i,e_i,i_i,a_i,\pi_i,\epsilon_i,\chi_i,m_i)
\]

- \(v\)：激活势；
- \(e\)：兴奋驱动；
- \(i\)：抑制驱动；
- \(a\)：适应；
- \(\pi\)：精度/增益；
- \(\epsilon\)：预测误差；
- \(\chi\)：资格迹聚合；
- \(m\)：代谢/计算储备。

节点以 SoA 定点数组保存，不创建 16K Python/Rust heap 对象。

## 动态稀疏图

\[
\mathcal G=(V,E,\tau,w,\xi,s,d)
\]

每条突触保存：目标、算子类型、权重、资格迹、结构稳定度、使用 epoch、延迟类和标志。

```text
初始边数    ≈ 262,144
硬上限      = 524,288
平均出度    16–32
```

## 群论与规范结构

### 区域内置换等变

同类节点重标号不应改变宏观输出：

\[
F(PH,PGP^{-1})=PF(H,G)
\]

### 共享 OperatorBank

每条边不保存完整矩阵：

\[
U_{ij}=a_{ij}U_{\tau_{ij}}
\]

最多 16 类低维共享算子：局部兴奋、抑制、显著广播、认知冲突、社会靠近、边界抑制、修复耦合、全局广播等。

### 内部曲率

沿环路运输：

\[
\Omega_C=U_{12}U_{23}\cdots U_{k1}
\]

\[
\kappa_C=\|I-\Omega_C\|
\]

非零曲率表示不同认知/情感路径无法在局部坐标中同时消解，例如“我确实错了”与“对方语气越界”并存。

## 动力学

采用离散梯度 Port-Hamiltonian 形式：

\[
M\frac{H_{n+1}-H_n}{\Delta t}
=(J-R)\bar\nabla\mathcal H(H_n,H_{n+1})+B u_n
\]

其中：

- \(J^T=-J\) 负责可逆交换；
- \(R\succeq0\) 负责抑制、疲劳和耗散；
- \(\mathcal H\) 由人格、身体、关系和残余决定；
- \(u_n\) 来自微型注意力装配的广义荷载。

无输入时：

\[
\mathcal H_{n+1}-\mathcal H_n
=-\Delta t\,\bar\nabla\mathcal H^TR\bar\nabla\mathcal H\le0
\]

因此不能凭空形成 warmth 自激泵。

## MVP 验收

- 16,384 槽位固定；
- 边数永不超过容量；
- 无输入能量不增；
- 节点重标号测试保持宏观 action digest；
- 1C1G 与 2C2G 得到相同神经 state digest；
- 无单个 `warmth_node`、`anger_node` 或 `disease_node`。
