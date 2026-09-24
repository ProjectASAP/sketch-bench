# VLDB/PVLDB Benchmark-Related Papers, 2021-2025

Scope: VLDB conference years 2021-2025, mapped to PVLDB Vol.14-18. The list uses a conservative inclusion rule: the paper title contains Bench/Benchmark/TPC/EAB, or the PVLDB abstract explicitly introduces a benchmark, benchmarking framework, benchmark suite, or evaluation suite. General system papers that merely run experiments on standard benchmarks are excluded.

Sources used: PVLDB official volume pages for Vol.14, Vol.15, Vol.16, Vol.17, and Vol.18; DBLP PVLDB search metadata; official PVLDB PDF URLs. All 58 PDF URLs below returned HTTP 200 during local verification.

## High-Level Summary

Across these five VLDB years, benchmark-related papers moved from classic database structures and workload design toward broader data-centric AI evaluation. VLDB 2021-2022 still contains many database-system benchmarks: learned indexes, decision support, procedural SQL workloads, trusted-execution joins, cardinality estimation, social-network workloads, persistent memory, and data matching. By 2023-2025, the center of gravity expands: time-series analysis, data lakes, graph learning, vector databases, NL2SQL, LLM-assisted benchmark generation, federated graph learning, data quality, fairness, privacy, and benchmark publishing all become recurring topics.

Two strong trends stand out. First, benchmark papers increasingly argue that old fixed workloads are insufficient for modern production systems, especially cloud analytics and HTAP systems; papers such as Cloud Analytics Benchmark, Why TPC Is Not Enough, PBench, SQLStorm, and Privacy-Enhanced Database Synthesis all address this gap from different angles. Second, benchmark construction is becoming a research contribution in its own right for AI/data-management tasks: TSGBench, TFB, TAB, TSB-AutoAD, BigVectorBench, LakeBench, ScienceBenchmark, OpenFGL, and VecCity all package datasets, task definitions, metrics, and tooling to make comparisons more reproducible.

## Papers

### VLDB 2021 / PVLDB Vol.14

1. **Benchmarking Learned Indexes**  
   URL: https://www.vldb.org/pvldb/vol14/p1-marcus.pdf  
   Summary: Establishes a unified benchmark comparing learned indexes with tuned traditional indexes, clarifying where learned models help and where their operational costs or assumptions dominate.

2. **CBench: Towards Better Evaluation of Question Answering Over Knowledge Graphs**  
   URL: https://www.vldb.org/pvldb/vol14/p1325-orogat.pdf  
   Summary: Introduces CBench for analyzing KGQA benchmarks and evaluating QA systems along more fine-grained linguistic, syntactic, structural, and performance dimensions.

3. **Procedural Extensions of SQL: Understanding their usage in the wild**  
   URL: https://www.vldb.org/pvldb/vol14/p1378-ramachandra.pdf  
   Summary: Studies real procedural SQL workloads and introduces SQL-ProcBench, a benchmark for procedural workloads in relational DBMSs.

4. **GraphMineSuite: Enabling High-Performance and Programmable Graph Mining Algorithms with Set Algebra**  
   URL: https://www.vldb.org/pvldb/vol14/p1922-besta.pdf  
   Summary: Presents GraphMineSuite, a benchmark and programmable framework for graph mining algorithms built around set-algebra primitives.

5. **Exathlon: A Benchmark for Explainable Anomaly Detection over Time Series**  
   URL: https://www.vldb.org/pvldb/vol14/p2613-tatbul.pdf  
   Summary: Provides a benchmark for explainable anomaly detection over high-dimensional time series, emphasizing realistic anomaly scenarios and explanation quality.

6. **CBench: Demonstrating Comprehensive Evaluation of Question Answering Systems over Knowledge Graphs Through Deep Analysis of Benchmarks**  
   URL: https://www.vldb.org/pvldb/vol14/p2711-orogat.pdf  
   Summary: Demonstrates the CBench platform for deeper analysis of KGQA benchmarks and system behavior beyond single aggregate metrics.

7. **A Demonstration of the Exathlon Benchmarking Platform for Explainable Anomaly Detection**  
   URL: https://www.vldb.org/pvldb/vol14/p2827-jacob.pdf  
   Summary: Demonstrates the Exathlon platform and workflow for evaluating explainable anomaly detection methods on time-series data.

8. **DSB: A Decision Support Benchmark for Workload-Driven and Traditional Database Systems**  
   URL: https://www.vldb.org/pvldb/vol14/p3376-ding.pdf  
   Summary: Defines a decision-support benchmark aimed at both traditional database systems and workload-driven learned/automated systems.

### VLDB 2022 / PVLDB Vol.15

9. **What Is the Price for Joining Securely? Benchmarking Equi-Joins in Trusted Execution Environments**  
   URL: https://www.vldb.org/pvldb/vol15/p659-maliszewski.pdf  
   Summary: Benchmarks equi-join execution in trusted execution environments, quantifying the cost of secure joins under different design choices.

10. **Cardinality Estimation in DBMS: A Comprehensive Benchmark Evaluation**  
    URL: https://www.vldb.org/pvldb/vol15/p752-zhu.pdf  
    Summary: Comprehensively evaluates cardinality estimation methods in DBMS settings and studies their downstream impact on query plans.

11. **ForBackBench: A Benchmark for Chasing vs. Query-Rewriting**  
    URL: https://www.vldb.org/pvldb/vol15/p1519-alhazmi.pdf  
    Summary: Provides a benchmark for comparing chase-based and query-rewriting approaches in data integration, exchange, and ontology-based access.

12. **TSB-UAD: An End-to-End Benchmark Suite for Univariate Time-Series Anomaly Detection**  
    URL: https://www.vldb.org/pvldb/vol15/p1697-paparrizos.pdf  
    Summary: Offers an end-to-end suite for univariate time-series anomaly detection, covering datasets, algorithms, and evaluation methodology.

13. **TAOBench: An End-to-End Benchmark for Social Networking Workloads**  
    URL: https://www.vldb.org/pvldb/vol15/p1965-cheng.pdf  
    Summary: Models large social-networking workloads inspired by TAO-like object and association services for end-to-end system evaluation.

14. **PerMA-Bench: Benchmarking Persistent Memory Access**  
    URL: https://www.vldb.org/pvldb/vol15/p2463-benson.pdf  
    Summary: Benchmarks persistent-memory access patterns to expose performance differences relevant to database storage design.

15. **Frost: A Platform for Benchmarking and Exploring Data Matching Results**  
    URL: https://www.vldb.org/pvldb/vol15/p3292-panse.pdf  
    Summary: Provides an interactive platform for comparing, exploring, and diagnosing data matching and entity-resolution results.

16. **SmartBench: Demonstrating Automatic Generation of Comprehensive Benchmarks for Question Answering Over Knowledge Graphs**  
    URL: https://www.vldb.org/pvldb/vol15/p3662-orogat.pdf  
    Summary: Demonstrates automatic generation of more comprehensive KGQA benchmarks to improve coverage and diagnostic power.

17. **TimeEval: A Benchmarking Toolkit for Time Series Anomaly Detection Algorithms**  
    URL: https://www.vldb.org/pvldb/vol15/p3678-schmidl.pdf  
    Summary: Introduces a toolkit for running reproducible and comparable evaluations of time-series anomaly detection algorithms.

### VLDB 2023 / PVLDB Vol.16

18. **M2Bench: A Database Benchmark for Multi-Model Analytic Workloads**  
    URL: https://www.vldb.org/pvldb/vol16/p747-moon.pdf  
    Summary: Defines analytic workloads spanning multiple data models, targeting systems that go beyond a single relational or graph workload.

19. **The LDBC Social Network Benchmark: Business Intelligence Workload**  
    URL: https://www.vldb.org/pvldb/vol16/p877-szarnyas.pdf  
    Summary: Finalizes the LDBC SNB BI workload for graph-oriented analytical systems using social-network data and business-intelligence queries.

20. **Cloud Analytics Benchmark**  
    URL: https://www.vldb.org/pvldb/vol16/p1413-renen.pdf  
    Summary: Proposes a benchmark for cloud-native analytics, reflecting service-oriented cloud deployment and evaluation concerns.

21. **Benchmarking the Utility of 𝑤-event Differential Privacy Mechanisms - When Baselines Become Mighty Competitors**  
    URL: https://www.vldb.org/pvldb/vol16/p1830-schaler.pdf  
    Summary: Benchmarks w-event differential privacy mechanisms and shows that strong baselines can be highly competitive in utility.

22. **Pollock: A Data Loading Benchmark**  
    URL: https://www.vldb.org/pvldb/vol16/p1870-vitagliano.pdf  
    Summary: Targets data loading, especially CSV-style ingestion, as a first-class benchmark problem for data systems.

23. **VeriBench: Analyzing the Performance of Database Systems with Verifiability**  
    URL: https://www.vldb.org/pvldb/vol16/p2145-ooi.pdf  
    Summary: Introduces a benchmark framework for verifiability-enabled database systems and compares design choices for authenticated query processing.

24. **TSM-Bench: Benchmarking Time Series Database Systems for Monitoring Applications**  
    URL: https://www.vldb.org/pvldb/vol16/p3363-khelifati.pdf  
    Summary: Benchmarks time-series database systems under monitoring workloads with realistic write and query patterns.

25. **CDSBen: Benchmarking the Performance of Storage Services in Cloud-native Database System at ByteDance**  
    URL: https://www.vldb.org/pvldb/vol16/p3584-tang.pdf  
    Summary: Benchmarks storage-service performance inside cloud-native database architectures, based on ByteDance production experience.

26. **FEBench: A Benchmark for Real-Time Relational Data Feature Extraction**  
    URL: https://www.vldb.org/pvldb/vol16/p3597-lu.pdf  
    Summary: Defines a benchmark for real-time feature extraction from relational data, supporting online ML inference scenarios.

27. **TPCx-AI - An Industry Standard Benchmark for Artificial Intelligence and Machine Learning Systems**  
    URL: https://www.vldb.org/pvldb/vol16/p3649-rabl.pdf  
    Summary: Presents the TPCx-AI industry benchmark for evaluating AI and ML systems across training and inference workflows.

### VLDB 2024 / PVLDB Vol.17

28. **TSGBench: Time Series Generation Benchmark**  
    URL: https://www.vldb.org/pvldb/vol17/p305-huang.pdf  
    Summary: Benchmarks synthetic time-series generation methods with tasks and metrics that cover generation quality and downstream utility.

29. **ScienceBenchmark: A Complex Real-World Benchmark for Evaluating Natural Language to SQL Systems**  
    URL: https://www.vldb.org/pvldb/vol17/p685-stockinger.pdf  
    Summary: Provides a realistic scientific-data NL2SQL benchmark with more complex schemas and queries than common textbook-style datasets.

30. **HyBench: A New Benchmark for HTAP Databases**  
    URL: https://www.vldb.org/pvldb/vol17/p939-zhang.pdf  
    Summary: Introduces an HTAP benchmark combining transactional and analytical behavior in a representative mixed workload.

31. **Text-to-SQL Empowered by Large Language Models: A Benchmark Evaluation**  
    URL: https://www.vldb.org/pvldb/vol17/p1132-gao.pdf  
    Summary: Evaluates LLM-based text-to-SQL systems and highlights capability, cost, and reliability gaps.

32. **OEBench: Investigating Open Environment Challenges in Real-World Relational Data Streams**  
    URL: https://www.vldb.org/pvldb/vol17/p1283-diao.pdf  
    Summary: Introduces OEBench for real-world relational data streams where distributions, classes, and features can change over time.

33. **How do Categorical Duplicates Affect ML? A New Benchmark and Empirical Analyses**  
    URL: https://www.vldb.org/pvldb/vol17/p1391-shah.pdf  
    Summary: Builds a benchmark to study how categorical duplicates in data preparation affect ML model behavior.

34. **FCBench: Cross-Domain Benchmarking of Lossless Compression for Floating-point Data**  
    URL: https://www.vldb.org/pvldb/vol17/p1418-tao.pdf  
    Summary: Compares floating-point lossless compression methods across database and HPC contexts using cross-domain datasets.

35. **LakeBench: A Benchmark for Discovering Joinable and Unionable Tables in Data Lakes**  
    URL: https://www.vldb.org/pvldb/vol17/p1925-chai.pdf  
    Summary: Benchmarks algorithms that discover joinable and unionable tables in poorly maintained data lakes.

36. **BYO: A Unified Framework for Benchmarking Large-Scale Graph Containers**  
    URL: https://www.vldb.org/pvldb/vol17/p2307-wheatman.pdf  
    Summary: Provides a unified framework for benchmarking graph containers, the core data structures underlying graph algorithms.

37. **TFB: Towards Comprehensive and Fair Benchmarking of Time Series Forecasting Methods**  
    URL: https://www.vldb.org/pvldb/vol17/p2363-hu.pdf  
    Summary: Builds a comprehensive and fair benchmark for time-series forecasting methods across datasets and evaluation settings.

38. **The Dawn of Natural Language to SQL: Are We Fully Ready? [Experiment, Analysis & Benchmark ]**  
    URL: https://www.vldb.org/pvldb/vol17/p3318-luo.pdf  
    Summary: Analyzes NL2SQL readiness under the EAB track, focusing on data, model, and evaluation limitations in the LLM era.

39. **A Benchmark Study of Deep-RL Methods for Maximum Coverage Problems over Graphs**  
    URL: https://www.vldb.org/pvldb/vol17/p3666-ke.pdf  
    Summary: Benchmarks deep reinforcement learning methods for graph maximum coverage problems and related combinatorial optimization tasks.

40. **Why TPC Is Not Enough: An Analysis of the Amazon Redshift Fleet**  
    URL: https://www.vldb.org/pvldb/vol17/p3694-saxena.pdf  
    Summary: Uses Amazon Redshift fleet analysis to show gaps between standard TPC benchmarks and real production analytical workloads.

41. **TSGAssist: An Interactive Assistant Harnessing LLMs and RAG for Time Series Generation Recommendations and Benchmarking**  
    URL: https://www.vldb.org/pvldb/vol17/p4309-huang.pdf  
    Summary: Demonstrates an assistant that uses LLMs and retrieval to recommend and benchmark time-series generation methods.

42. **ImputeVIS: An Interactive Evaluator to Benchmark Imputation Techniques for Time Series Data**  
    URL: https://www.vldb.org/pvldb/vol17/p4329-khayati.pdf  
    Summary: Provides an interactive evaluator for benchmarking time-series missing-data imputation techniques.

43. **SEER: An End-to-End Toolkit for Benchmarking Time Series Database Systems in Monitoring Applications**  
    URL: https://www.vldb.org/pvldb/vol17/p4361-khayati.pdf  
    Summary: Demonstrates an end-to-end toolkit for benchmarking time-series databases in monitoring scenarios.

### VLDB 2025 / PVLDB Vol.18

44. **Privacy-Enhanced Database Synthesis for Benchmark Publishing**  
    URL: https://www.vldb.org/pvldb/vol18/p413-zheng.pdf  
    Summary: Studies how to publish benchmark-like databases while preserving privacy and retaining workload utility.

45. **How Reliable Are Streams? End-to-End Processing-Guarantee Validation and Performance Benchmarking of Stream Processing Systems**  
    URL: https://www.vldb.org/pvldb/vol18/p585-tahir.pdf  
    Summary: Combines reliability validation and performance benchmarking for stream processing systems under end-to-end guarantees.

46. **The ParClusterers Benchmark Suite (PCBS): A Fine-Grained Analysis of Scalable Graph Clustering**  
    URL: https://www.vldb.org/pvldb/vol18/p836-yu.pdf  
    Summary: Introduces a benchmark suite and tools for fine-grained comparison of scalable graph clustering algorithms.

47. **OpenFGL: A Comprehensive Benchmark for Federated Graph Learning**  
    URL: https://www.vldb.org/pvldb/vol18/p1305-li.pdf  
    Summary: Provides datasets, tasks, and methods for evaluating federated graph learning under distributed data constraints.

48. **BigVectorBench: Heterogeneous Data Embedding and Compound Queries are Essential in Evaluating Vector Databases**  
    URL: https://www.vldb.org/pvldb/vol18/p1536-zhan.pdf  
    Summary: Benchmarks vector databases using heterogeneous embeddings and compound queries, arguing that simple vector search is insufficient.

49. **VecCity: A Taxonomy-guided Library for Map Entity Representation Learning [Experiment, Analysis & Benchmark]**  
    URL: https://www.vldb.org/pvldb/vol18/p2575-wang.pdf  
    Summary: Provides a taxonomy-guided library and benchmark for representation learning over map entities such as roads and POIs.

50. **TAB: Unified Benchmarking of Time Series Anomaly Detection Methods**  
    URL: https://www.vldb.org/pvldb/vol18/p2775-hu.pdf  
    Summary: Unifies benchmarking for time-series anomaly detection methods, emphasizing fair comparison across models and datasets.

51. **The UDFBench Benchmark for General-purpose UDF Queries**  
    URL: https://www.vldb.org/pvldb/vol18/p2804-foufoulas.pdf  
    Summary: Defines UDFBench to evaluate database execution of general-purpose user-defined-function queries.

52. **Still More Shades of Null: An Evaluation Suite for Responsible Missing Value Imputation [Experiment, Analysis and Benchmark]**  
    URL: https://www.vldb.org/pvldb/vol18/p2899-stoyanovich.pdf  
    Summary: Introduces an evaluation suite for responsible missing-value imputation, considering quality, fairness, and downstream behavior.

53. **The LDBC Financial Benchmark: Transaction Workload**  
    URL: https://www.vldb.org/pvldb/vol18/p3007-qi.pdf  
    Summary: Defines the transaction workload for the LDBC Financial Benchmark, targeting graph-database behavior in financial applications.

54. **PBench: Workload Synthesizer with Real Statistics for Cloud Analytics Benchmarking**  
    URL: https://www.vldb.org/pvldb/vol18/p3883-fan.pdf  
    Summary: Synthesizes cloud analytics workloads using real statistics to better match production behavior than fixed standard benchmarks.

55. **SQLStorm: Taking Database Benchmarking into the LLM Era**  
    URL: https://www.vldb.org/pvldb/vol18/p4144-schmidt.pdf  
    Summary: Uses LLMs to construct and expand SQL benchmarks, introducing SQLStorm as a benchmark on real-world data at multiple scales.

56. **TSB-AutoAD: Towards Automated Solutions for Time-Series Anomaly Detection [E, A & B]**  
    URL: https://www.vldb.org/pvldb/vol18/p4364-liu.pdf  
    Summary: Benchmarks automated time-series anomaly detection solutions, including selection, ensembling, and generation methods.

57. **Time-Series Clustering: A Comprehensive Study of Data Mining, Machine Learning, and Deep Learning Methods**  
    URL: https://www.vldb.org/pvldb/vol18/p4380-paparrizos.pdf  
    Summary: Provides a broad empirical study of time-series clustering methods and highlights gaps in existing benchmarking practice.

58. **Benchmarking Adaptive Multidimensional Indices**  
    URL: https://www.vldb.org/pvldb/vol18/p4505-lampropoulos.pdf  
    Summary: Benchmarks adaptive multidimensional indexes under dynamic query processing and exploratory workloads.

## Validation Notes

- Research subagent output was used to catch items missed by DBLP keyword search, including CBench, VeriBench, BigVectorBench, TSB-AutoAD, and Why TPC Is Not Enough.
- Local verification checked every listed PVLDB PDF URL with `curl -L -I`; all returned HTTP 200.
- An independent verification subagent confirmed 53 benchmark-related papers by DBLP/PVLDB title/tag search and verified the listed official PVLDB URLs. It also flagged boundary cases caused by PVLDB's cross-year volumes and by demo/tool papers.
- The report intentionally keeps five broader-scope items that the independent verifier did not include in its 53-paper list: Procedural Extensions of SQL, GraphMineSuite, Why TPC Is Not Enough, TSB-AutoAD, and Time-Series Clustering. Each is in the official PVLDB volume and is included here because the PVLDB page/abstract explicitly frames it as introducing or analyzing a benchmark, benchmark suite, or benchmark practice.
- Some PVLDB/DBLP records append track labels such as `[Experiment, Analysis & Benchmark]`. These labels are preserved when they appear in the official PVLDB metadata, even when the PDF title page may omit the bracketed track label.
