"""Synthetic documents and meaning-only queries for the VFS-29b gate (VFS-29, "Gate before
building any interface").

Every document here was written for this benchmark; names, companies' customers, figures
and addresses are invented. TARGETS are the documents meaning-only queries must find;
NEGATIVES are documents of the same family-archive kinds that share words with the queries
but do not answer them, so BM25 has something plausible to rank above the target.

A meaning-only query shares no content keyword with its target document: `check.py`
verifies this mechanically with the shipped analyzer's rules (Snowball Spanish stem, then
accent fold) and, for English, the Snowball English stem too.
"""

# Languages of the meaning-only queries (closed set).
ES, EN = "es", "en"

# (file name, text)
TARGETS = {
    # ---------------------------------------------------------------- Spanish
    "es_factura_dentista.txt": """Clínica Dental Sonrisa Retiro, S.L.P. — CIF B-84123456
Factura simplificada n.º 2023/0418. Fecha: 7 de marzo de 2023.
Paciente: Marta Robles Iglesias.

Concepto                                   Importe
Limpieza bucal con ultrasonidos              45,00 €
Empaste composite pieza 36                   60,00 €
Endodoncia molar pieza 46 (tres conductos)  280,00 €
Radiografía periapical                       15,00 €
Total                                       400,00 €

Operación exenta de IVA según el artículo 20.Uno.3.º de la Ley 37/1992 (asistencia
sanitaria). Abonado con tarjeta. Conserve este documento: sirve como justificante para su
mutua o para la deducción autonómica si procede. Próxima revisión de higiene en seis
meses.""",
    "es_denuncia_radar.txt": """Dirección General de Tráfico — Jefatura Provincial de Toledo
Notificación de denuncia. Expediente 45-301-998.877-6.

Hecho denunciado: circular a 68 km/h en una vía urbana limitada a 50 km/h, captado por
cinemómetro fijo en la avenida de Europa, punto kilométrico 3,200, el día 14 de mayo de
2024 a las 18:42. Vehículo: turismo Seat León, matrícula 4471 KJT.

Sanción: 100 euros y pérdida de 2 puntos del permiso de conducción. Si abona el importe en
los 20 días naturales siguientes se le aplicará una reducción del 50 % (50 euros) y no
podrá presentar alegaciones. Puede identificar al conductor responsable si no era usted.""",
    "es_boletin_evaluacion.txt": """CEIP Miguel de Cervantes — Curso 2022-2023
Informe de la segunda evaluación. Alumna: Lucía Ortega Sanz. 4.º de Educación Primaria.

Lengua Castellana y Literatura: Sobresaliente (9)
Matemáticas: Notable (7)
Ciencias de la Naturaleza: Notable (8)
Inglés: Bien (6)
Educación Física: Sobresaliente (10)
Educación Artística: Notable (8)

Observaciones de la tutora: Lucía lee con fluidez y participa en clase. Debe repasar las
tablas de multiplicar y la división con dos cifras. Faltas de asistencia: 2, justificadas.""",
    "es_cartilla_inmunizacion.txt": """Servicio de Salud de Castilla y León — Calendario de inmunización infantil
Cartilla de Pablo Martín Gil, nacido el 2 de febrero de 2023. Centro de salud Arturo Eyries.

2 meses: hexavalente (difteria, tétanos, tosferina, polio, Haemophilus, hepatitis B),
neumococo conjugada 15-valente, meningococo B (1.ª dosis), rotavirus oral.
4 meses: hexavalente, neumococo, meningococo B (2.ª dosis), meningococo C.
11 meses: hexavalente, neumococo (refuerzo).
12 meses: triple vírica (sarampión, rubeola, parotiditis), meningococo ACWY,
meningococo B (refuerzo).

Pediatra: Dra. Alonso. Enfermera: Rosa. Sin reacciones adversas registradas en el lactante.
Próxima cita en la consulta de enfermería a los 15 meses: varicela.""",
    "es_reparacion_frigorifico.txt": """Servicio Técnico Oficial Balay — Parte de intervención n.º 77812
Fecha: 22 de julio de 2023. Domicilio: calle Arganzuela 14, 2.º A, Madrid.

Aparato: frigorífico combi Balay 3KFE563WI, n.º de serie 4410023.
Síntoma comunicado por el cliente: el compartimento inferior no enfría y el congelador
forma escarcha; el motor funciona sin parar.

Diagnóstico: fuga de gas refrigerante R600a en el evaporador y termostato defectuoso.
Trabajos realizados: sustitución del termostato, soldadura del circuito, vacío y carga de
gas. Mano de obra 1,5 h: 72,00 €. Desplazamiento: 25,00 €. Piezas: 48,50 €.
Total con IVA: 176,66 €. La intervención tiene tres meses de garantía.""",
    "es_factura_gas_enero.txt": """Naturgy Iberia, S.A. — Factura de gas natural
Periodo de facturación: del 12 de diciembre de 2022 al 13 de febrero de 2023 (63 días).
Titular: Ignacio Herrero Gómez. CUPS ES0230901000123456XY. Tarifa RL.2.

Lectura anterior: 10.214 m³. Lectura actual: 10.583 m³. Consumo: 369 m³ = 4.218 kWh.
Término fijo: 63 días × 0,2587 €/día = 16,30 €
Término variable: 4.218 kWh × 0,0810 €/kWh = 341,66 €
Impuesto de hidrocarburos: 9,19 €. Alquiler de contador: 1,89 €. IVA 5 %: 18,45 €.
Total: 387,49 €. Cargo en la cuenta ES12 2100 **** **** 4410 el 20 de febrero.

Su consumo diario medio ha sido de 67 kWh, frente a 9 kWh en el periodo de agosto.""",
    "es_parte_incapacidad.txt": """Instituto Nacional de la Seguridad Social — Parte médico de incapacidad temporal
Contingencia común. Fecha de inicio: 9 de enero de 2024.

Asegurado: Javier Soto Ramírez. Empresa: Logística Henares, S.L.
Diagnóstico: lumbalgia mecánica aguda con contractura paravertebral; molestias al
levantar peso y al permanecer sentado. Exploración neurológica normal.
Duración estimada del proceso: 15 días (proceso de corta duración).

Recomendaciones: reposo relativo, calor local, ibuprofeno 600 mg cada 8 horas durante
cinco días y ejercicios de estiramiento. Revisión en la consulta de atención primaria el
día 16 de enero. Entregue la copia correspondiente a su empresa.""",
    "es_reserva_conil.txt": """Booking — Confirmación n.º 3841.227.905
Apartamentos Playa La Fontanilla, Conil de la Frontera (Cádiz).

Llegada: sábado 3 de agosto de 2024, a partir de las 16:00.
Salida: sábado 17 de agosto de 2024, hasta las 11:00.
Apartamento de dos dormitorios con terraza, 4 huéspedes (2 adultos, 2 menores).
A 200 metros de la arena. Piscina comunitaria y plaza de garaje incluidas.

Precio total: 1.890 €. Se ha cobrado un depósito de 400 €; el resto se abona a la
llegada. Cancelación gratuita hasta el 20 de julio. Al llegar, presente el DNI de todos los
adultos.""",
    "es_aceptacion_adjudicacion.txt": """Notaría de don Fernando Sáez Molina — Valladolid
Escritura de aceptación y adjudicación de bienes. Número 1.204 de protocolo, 2021.

Causante: don Eusebio Molina Pardo, fallecido en Valladolid el 3 de noviembre de 2020,
viudo, que otorgó disposición de última voluntad el 12 de mayo de 2015 ante este notario.
Comparecen sus tres hijos, que aceptan pura y simplemente y se adjudican por partes
iguales: una vivienda en la calle Santiago 22, una finca rústica en Tudela de Duero y los
saldos de dos cuentas bancarias. Se liquidó el impuesto de sucesiones y donaciones en la
Junta de Castilla y León con las bonificaciones del grupo II.""",
    "es_alta_suministro_electrico.txt": """Iberdrola Clientes, S.A.U. — Condiciones particulares del suministro
Solicitud de alta con la nueva comercializadora. Fecha de efecto: 1 de octubre de 2023.

Titular: Ana Belén Prieto Lozano. Dirección de suministro: calle Olmo 8, 1.º B, Getafe.
CUPS: ES0021000012345678AB. Distribuidora: i-DE Redes Eléctricas Inteligentes.
Tarifa de acceso 2.0TD. Potencia punta 4,4 kW; potencia valle 4,4 kW.
Precio de la energía: 0,1390 €/kWh en todos los periodos (Plan Estable), sin permanencia.

La anterior comercializadora recibirá la baja automáticamente; no necesita hacer ningún
trámite. Le enviaremos las facturas cada mes por correo electrónico.""",
    "es_factura_tanatorio.txt": """Servicios Funerarios La Paz — Tanatorio de Alcorcón
Factura n.º F-2022-1187. Fecha: 19 de septiembre de 2022.
Difunta: doña Carmen Ruiz Esteban. Contratante: Luis Ruiz Morales.

Arca de pino barnizado con herrajes                 1.150,00 €
Sala de velatorio (24 horas)                          420,00 €
Coche fúnebre y traslado al cementerio               310,00 €
Corona de flores naturales                            180,00 €
Inhumación en el cementerio municipal                 390,00 €
Esquelas en prensa                                    120,00 €
Total (IVA incluido)                                2.570,00 €

Parte de este importe lo cubre la póliza de decesos de Santalucía n.º 88-12345.""",
    "es_informe_optica.txt": """Centro Óptico Alameda — Informe de graduación
Paciente: Daniel Fuentes Rico, 8 años. Fecha: 15 de noviembre de 2023.

Motivo de consulta: no distingue lo que se escribe en la pizarra y entorna los ojos al leer
de lejos. Agudeza visual sin corrección: OD 0,5; OI 0,6.
Refracción: OD −1,75 esf −0,50 cil 180°; OI −1,50 esf. Miopía con astigmatismo leve.
Recomendación: lentes monofocales de policarbonato para uso permanente, montura flexible,
y control cada seis meses por posible progresión. Limitar pantallas y favorecer actividad
al aire libre. Si aparecen dolores de cabeza, acudir al oftalmólogo.""",
    "es_actualizacion_renta.txt": """Fincas Martínez Arrieta — Comunicación al inquilino
Madrid, 1 de febrero de 2023.

Estimada señora Navarro:
Conforme a la cláusula cuarta de su contrato de arrendamiento, le comunicamos la
actualización anual de la renta según el índice de referencia, con el límite del 2 %
establecido por el Real Decreto-ley 6/2022. La mensualidad pasa de 750,00 € a 765,00 € a
partir del recibo del 1 de marzo de 2023. El resto de condiciones no cambian.

Atentamente, Pilar Arrieta, administradora.""",
    "es_resolucion_beca.txt": """Ministerio de Educación y Formación Profesional
Resolución de la convocatoria de becas de carácter general, curso 2023-2024.

Solicitante: Irene Campos Delgado. Estudios: Grado en Enfermería (Salamanca).
Componentes concedidos:
- Cuantía fija ligada a la renta del solicitante: 1.700 €
- Cuantía fija ligada a la residencia durante el curso: 2.500 €
- Cuantía variable: 612,40 €
- Matrícula gratuita en los créditos de primera matrícula.

El importe se ingresará en la cuenta indicada en la solicitud. Deberá superar el 65 % de los
créditos matriculados para no tener que reintegrarlo.""",
    "es_denuncia_sustraccion.txt": """Policía Municipal de Madrid — Atestado n.º 2023-004512
Comparecencia por sustracción. Fecha: 30 de abril de 2023, 21:15 h.

Comparece don Álvaro Peña Ruiz, quien manifiesta que entre las 19:00 y las 20:30 horas,
alguien se llevó de la entrada del edificio de la calle Embajadores 101 su vehículo de dos
ruedas sin motor, marca Orbea, modelo Vector 30, color azul, de ruedas de 28 pulgadas,
n.º de cuadro WOR7781123, que había dejado atado a la farola con un candado en U que
aparece cortado. Valor aproximado: 650 €. Aporta factura de adquisición.""",
    "es_parte_filtracion.txt": """Mapfre Hogar — Declaración de siniestro n.º 2024-77-001234
Fecha de ocurrencia: 2 de febrero de 2024.

Descripción: desde hace varias semanas aparecen manchas oscuras y moho en el techo y los
azulejos del aseo, junto a la ducha. El fontanero de la compañía localiza una filtración
de agua procedente de la tubería de desagüe de la vivienda superior (4.º B).
Daños: pintura y yeso del techo deteriorados, junta de azulejos levantada, armario de
madera hinchado. Se tramita como daños por agua con responsabilidad del vecino de arriba;
el perito visitará la vivienda el día 9.""",
    "es_analitica_lipidos.txt": """Laboratorio de Análisis Clínicos — Hospital Universitario La Princesa
Paciente: Ramón Iglesias Pérez, 58 años. Fecha de extracción: 4 de octubre de 2023.

Perfil lipídico (en ayunas)                Resultado      Valores de referencia
LDL                                         182 mg/dL      < 116 mg/dL
HDL                                          41 mg/dL      > 40 mg/dL
Triglicéridos                               210 mg/dL      < 150 mg/dL
No-HDL                                      224 mg/dL      < 145 mg/dL

Comentario del médico de familia: lípidos claramente elevados con riesgo cardiovascular
moderado. Iniciar atorvastatina 20 mg, reducir grasas saturadas, caminar 30 minutos al día
y repetir el perfil en tres meses.""",
    "es_resolucion_supervivencia.txt": """Instituto Nacional de la Seguridad Social — Dirección Provincial de Sevilla
Resolución sobre prestación por muerte y supervivencia.

Beneficiaria: doña Rosario Gil Romero, cónyuge superviviente de don Antonio Vega Luna,
fallecido el 11 de junio de 2023.
Se reconoce el derecho con efectos del 12 de junio de 2023. Base reguladora: 1.842,30 €
mensuales. Porcentaje aplicable: 52 %. Cuantía inicial: 958,00 € mensuales en catorce
pagas, revalorizable cada año. Se le abonará también el auxilio por defunción de 46,50 €.
Contra esta resolución puede presentar reclamación previa en el plazo de 30 días.""",
    "es_alta_linea_portabilidad.txt": """Movistar — Resumen de condiciones de su línea
Fecha de alta: 5 de septiembre de 2023. Titular: Sergio Lozano Vidal.

Portabilidad del número 6** *** 412 desde Vodafone. Tarifa: 30 GB, llamadas ilimitadas,
15,90 €/mes. Terminal: Samsung Galaxy A54 financiado en 24 cuotas de 18,75 €.
Compromiso de permanencia: 12 meses; si se da de baja antes, penalización proporcional de
hasta 120 €. Tarjeta SIM enviada al domicilio; actívela en las 48 horas siguientes a la
recepción. Atención al cliente: 1004.""",
    "es_escuela_infantil.txt": """Escuela Infantil Municipal Los Pinos — Matrícula curso 2023-2024
Unidad: 1-2 años. Alumno: Hugo Serrano Blanco, nacido el 3 de marzo de 2022.

Horario: de 9:00 a 16:00, con servicio de desayuno a partir de las 7:30.
Cuota mensual de escolaridad: 160 € (tramo de renta C). Comida: 96 € al mes.
Material: 60 € al año, en septiembre.
Período de adaptación: del 4 al 15 de septiembre, con horario reducido.
Deben traer muda completa, pañales, toallitas y una foto. Las educadoras de la unidad son
Laura y Begoña.""",
    "es_compraventa_vehiculo.txt": """Contrato de compraventa de vehículo usado entre particulares
En Zaragoza, a 18 de junio de 2024.

Vendedor: don Miguel Ángel Tena Sanz. Adquirente: doña Elena Royo Lahoz.
Objeto: turismo Volkswagen Golf 1.6 TDI, matrícula 9182 HBN, bastidor
WVWZZZ1KZ9W123456, con 148.300 kilómetros. Precio: 6.200 €, abonados por transferencia.
El vendedor declara que el vehículo está al corriente de la ITV y del impuesto de
circulación y libre de cargas. La adquirente se encarga del cambio de titularidad en la
DGT y del impuesto de transmisiones patrimoniales.""",
    "es_plan_nutricional.txt": """Consulta de Nutrición Clínica — Plan hipocalórico personalizado
Paciente: Beatriz Lara Muñoz. Peso inicial: 86 kg. Objetivo: 74 kg en seis meses.

Desayuno: café con leche desnatada, una tostada integral con tomate y aceite.
Media mañana: una pieza de fruta.
Comida: verdura, 120 g de pescado o pollo a la plancha, una pieza de fruta.
Merienda: yogur natural sin azúcar.
Cena: crema de verduras y tortilla francesa de un huevo.
Evitar bollería, refrescos y fritos. Beber 1,5 litros de agua al día. Perder unos 0,5 kg
por semana es un ritmo saludable. Próxima consulta en un mes.""",
    "es_manual_friegaplatos.txt": """Manual de uso — Bosch Serie 4 SMS4HVI33E
Antes del primer uso, llene el depósito de sal regeneradora (aprox. 1,5 kg) y el de
abrillantador.

1. Coloque las ollas y sartenes en la cesta inferior, con la abertura hacia abajo.
2. Los vasos y tazas van en la cesta superior; los cubiertos, en la bandeja de arriba.
3. Ponga una pastilla en el compartimento del detergente y cierre la tapa.
4. Elija el programa: Eco 50 °C para la carga diaria, Intensivo 70 °C para suciedad
   incrustada, Rápido 45 °C para piezas poco sucias.
5. Pulse inicio. Al terminar, espere unos minutos antes de abrir la puerta.
Limpie el filtro del fondo una vez al mes.""",
    "es_poliza_automovil.txt": """Línea Directa Aseguradora — Condiciones particulares
Póliza n.º 12-3456789. Modalidad: todo riesgo con franquicia de 300 €.
Tomador: Rubén Castro Marín. Vehículo: Toyota Corolla Hybrid, matrícula 2210 LMN.

Coberturas: responsabilidad civil obligatoria y voluntaria hasta 50 millones de euros,
lunas, incendio, sustracción, daños propios con franquicia, asistencia en carretera desde
el kilómetro 0, vehículo de sustitución 15 días y defensa jurídica.
Prima anual: 512,30 €. Efecto: 1 de abril de 2024. Vencimiento: 1 de abril de 2025.
Recuerde comunicar cualquier siniestro en un plazo máximo de siete días.""",
    "es_recibo_cuotas_finca.txt": """Administración de Fincas Gómez — Recibo de cuotas ordinarias
Finca: calle Bravo Murillo 210, Madrid. Piso: 3.º B. Propietario: Andrés Molina.
Trimestre: abril–junio de 2024.

Cuota ordinaria trimestral (coeficiente 2,85 %): 126,40 €
Fondo de reserva: 12,64 €
Total: 139,04 €. Se cargará en su cuenta el día 5 del mes.

Incluye limpieza de portal y escaleras, mantenimiento del ascensor, luz del portal y el garaje,
seguro del edificio y honorarios de la administración. La próxima junta ordinaria será en
octubre.""",
    "es_informe_cardiologia.txt": """Servicio de Cardiología — Hospital Clínico San Carlos
Informe de consulta externa. Paciente: Josefa Martín Ortiz, 71 años.

Motivo: palpitaciones y sensación de latido irregular desde hace dos meses.
Electrocardiograma: fibrilación auricular con respuesta ventricular a 98 lpm.
Ecocardiograma: fracción de eyección conservada (60 %), aurícula izquierda ligeramente
dilatada. Holter de 24 horas: episodios de fibrilación paroxística.
Tratamiento: apixabán 5 mg cada 12 horas y bisoprolol 2,5 mg al día.
Próxima revisión en esta consulta el 12 de mayo, con nuevo Holter.""",
    # ---------------------------------------------------------------- English
    "en_dental_invoice.txt": """Bright Smile Dental Practice — Invoice 2023-0912
Date: 14 September 2023. Patient: Oliver Hughes.

Scale and polish                               £65.00
Composite filling, lower left molar           £120.00
Root canal treatment, upper right premolar    £390.00
Bitewing X-rays                                £30.00
Total due                                     £605.00

Paid by debit card on the day. Please keep this receipt for your health cash plan claim.
Your next hygiene appointment is due in six months.""",
    "en_camera_notice.txt": """Thames Valley Police — Notice of Intended Prosecution
Reference TVP/2024/556120. Vehicle: Ford Focus, registration KX19 ABC.

On 3 May 2024 at 07:52 the vehicle was recorded by a fixed safety camera on the A4074
travelling at 41 mph where the limit is 30 mph.
You may accept a fixed penalty of £100 and three points on your licence, or be offered a
course if eligible. You must name the driver within 28 days. Failure to do so is an offence.""",
    "en_pupil_report.txt": """Hillside Primary — Pupil Progress Report, Spring Term
Pupil: Amelia Clarke. Year 4. Class teacher: Mr Patel.

Reading: working at greater depth.
Writing: at the expected standard; needs to use paragraphs more consistently.
Maths: working towards the expected standard; practise times tables at home.
Science: at the expected standard. PE: excellent effort.
Attendance: 97 %. Amelia is kind and helpful and contributes well in class discussions.""",
    "en_engineer_visit.txt": """Hotpoint Service — Engineer Visit Record 448120
Date: 2 August 2023. Appliance: fridge freezer, model H8 A2E W.

Customer reported: lower compartment not cooling, ice building up on the back wall,
compressor running constantly.
Diagnosis: faulty thermostat and blocked defrost drain.
Work carried out: thermostat replaced, drain cleared, unit tested for two hours.
Labour £85.00, parts £42.50, call-out £30.00. Total £157.50 including VAT.
Parts and labour guaranteed for 90 days.""",
    "en_lipid_panel.txt": """Riverside Health Centre — Laboratory Results
Patient: George Whitman, 61. Sample taken: 9 October 2023 (fasting).

LDL 4.9 mmol/L (target below 3.0)
HDL 1.0 mmol/L
Triglycerides 2.3 mmol/L (target below 1.7)
Non-HDL 5.6 mmol/L

GP comment: lipids are clearly raised and ten-year cardiovascular risk is 14 %. Start
atorvastatin 20 mg at night, cut down on saturated fat, and repeat bloods in three months.""",
    "en_tenancy_notice.txt": """Parkside Lettings — Notice to Tenant
Property: 12 Elm Grove, Leeds LS6. Tenant: Ms Hannah Wright. Date: 1 February 2024.

In line with clause 7 of your tenancy agreement, we write to let you know that the monthly
payment for the property will change from £950 to £1,000 per calendar month, with effect
from 1 April 2024. All other terms remain the same. Please update your standing order
before that date.""",
    "en_immunisation_record.txt": """NHS Personal Child Health Record — Immunisations
Child: Noah Evans, born 4 January 2023. GP surgery: Oakfield Medical Practice.

8 weeks: 6-in-1 (diphtheria, tetanus, whooping cough, polio, Hib, hepatitis B),
MenB, rotavirus.
12 weeks: 6-in-1, pneumococcal, rotavirus.
16 weeks: 6-in-1, MenB.
1 year: Hib/MenC, MMR (measles, mumps, rubella), pneumococcal booster, MenB booster.
No reactions recorded. Next: pre-school booster at 3 years 4 months.""",
    "en_motor_policy.txt": """Direct Line — Your Motor Policy Schedule
Policy number 77-1234567. Policyholder: Daniel Moore. Vehicle: Honda Civic, AB21 XYZ.

Cover: comprehensive. Compulsory excess £250, voluntary excess £100.
No-claims discount: 5 years, protected. Courtesy vehicle included. Windscreen cover included.
Period of cover: 12 March 2024 to 11 March 2025. Annual premium £486.20, paid monthly by
direct debit. Please tell us about any changes to your details or the vehicle.""",
    "en_undertaker_invoice.txt": """Wilson & Sons Undertakers — Invoice 2022/341
Date: 5 October 2022. Deceased: Mrs Margaret Collins. Client: Peter Collins.

Oak veneer coffin                     £895.00
Hearse and one limousine              £560.00
Crematorium fee                       £1,050.00
Celebrant                             £220.00
Floral tribute                        £150.00
Newspaper notice                       £85.00
Total                                 £2,960.00

Payment is due within 28 days; a pre-paid plan reference may be quoted.""",
    "en_dishwasher_manual.txt": """Beko DVN05320W — User Manual
Before first use, fill the salt container and the rinse aid dispenser.

1. Load pots and pans in the lower basket, facing down.
2. Glasses and cups go in the upper basket; cutlery in the cutlery tray.
3. Put a tablet in the detergent compartment and close the lid.
4. Choose a programme: Eco 50 °C for daily loads, Intensive 70 °C for burnt-on food,
   Quick 35 °C for lightly soiled items.
5. Press start. Clean the filter at the bottom once a month.""",
}

NEGATIVES = {
    # ---------------------------------------------------------------- Spanish
    "es_neg_factura_agua.txt": """Canal de Isabel II — Factura de agua. Periodo: julio y agosto de 2023.
Consumo: 28 m³. Cuota de servicio: 14,20 €. Aducción, distribución y depuración: 41,30 €.
Total: 58,90 €. Domiciliado. Le recordamos que una fuga puede multiplicar su consumo.""",
    "es_neg_factura_fibra.txt": """Digi — Factura mensual de fibra y teléfono fijo. Octubre de 2023.
Fibra 1 Gb: 20,00 €. Línea fija con llamadas ilimitadas: incluida. Total: 20,00 €.
Si cambia de compañía, recuerde devolver el router en 30 días.""",
    "es_neg_comision_banco.txt": """Banco del Norte — Comunicación de cambio de condiciones
A partir del 1 de enero de 2024 la comisión de mantenimiento de su cuenta pasa a 8 € al
mes, salvo que domicilie una nómina o pensión superior a 600 €. Puede cancelar la cuenta
sin gastos en los dos meses siguientes.""",
    "es_neg_informe_trauma.txt": """Servicio de Traumatología — Informe médico
Paciente con dolor en la rodilla derecha tras torsión jugando al fútbol. Resonancia: rotura
del menisco interno. Se propone artroscopia. Mientras tanto, reposo deportivo, hielo y
fisioterapia. Puede trabajar en tareas sin carga.""",
    "es_neg_hemograma.txt": """Laboratorio — Hemograma y bioquímica básica
Hemoglobina 14,1 g/dL; leucocitos 6.800/µL; plaquetas 245.000/µL. Glucosa 92 mg/dL;
creatinina 0,9 mg/dL; hierro 85 µg/dL; ferritina 60 ng/mL. Todos los valores dentro de la
normalidad. Comentario: analítica normal, repetir en un año.""",
    "es_neg_circular_comedor.txt": """Colegio Nuestra Señora del Pilar — Circular a las familias
El servicio de comedor empieza el 11 de septiembre. Menú de octubre adjunto. Los niños
con alergias deben entregar el informe del pediatra en secretaría antes del día 15. El
precio del comedor para este curso es de 5,40 € al día.""",
    "es_neg_garantia_televisor.txt": """Garantía comercial — Televisor LG OLED 55 pulgadas
Fecha de compra: 12 de diciembre de 2022, MediaMarkt Alcalá. Tres años de garantía legal.
Para cualquier avería, conserve el ticket y contacte con el servicio técnico de LG.""",
    "es_neg_presupuesto_cocina.txt": """Reformas Hermanos García — Presupuesto de reforma de cocina
Demolición de alicatado, nueva instalación de fontanería y electricidad, muebles de cocina
en blanco mate, encimera de cuarzo, campana y placa de inducción. Total: 9.850 € más IVA.
Plazo de ejecución: tres semanas.""",
    "es_neg_multa_ora.txt": """Ayuntamiento de Madrid — Boletín de denuncia por estacionamiento
Multa por estacionar en zona SER sin ticket válido, calle Goya 45, 10:15 h. Importe: 90 €,
45 € si se paga en 20 días. No conlleva pérdida de puntos.""",
    "es_neg_billete_tren.txt": """Renfe — Billete de tren AVE Madrid Puerta de Atocha – Sevilla Santa Justa
Fecha: 2 de agosto de 2024, salida 08:00, llegada 10:32. Coche 5, plaza 9A. Tarifa Básico.
Precio: 47,30 €. Viaje de vacaciones con cambio permitido hasta 2 horas antes.""",
    "es_neg_receta_paracetamol.txt": """Receta médica electrónica — Sistema Nacional de Salud
Paracetamol 1 g, comprimidos. Posología: un comprimido cada 8 horas si dolor o fiebre,
máximo 5 días. Médico prescriptor: Dr. Hernández, centro de salud Las Calesas.""",
    "es_neg_empadronamiento.txt": """Ayuntamiento de Getafe — Volante de empadronamiento
Se certifica que en el domicilio de la calle Olmo 8, 1.º B, figuran inscritos en el padrón
municipal de habitantes tres personas desde el 4 de abril de 2019.""",
    "es_neg_alquiler_garaje.txt": """Contrato de alquiler de plaza de garaje
El propietario cede en alquiler la plaza n.º 14 del garaje de la calle Alcalá 300 por
85 € al mes. Duración: un año prorrogable. Fianza: una mensualidad.""",
    "es_neg_siniestro_joyas.txt": """Seguro de hogar — Parte de siniestro por robo
Durante unas vacaciones entraron en la vivienda forzando la puerta y se llevaron joyas y un
ordenador portátil. Se adjunta copia de la denuncia. Valor reclamado: 3.200 €.""",
    "es_neg_nomina.txt": """Nómina de marzo de 2024 — Logística Henares, S.L.
Trabajador: categoría de mozo de almacén. Salario base 1.210 €, plus de transporte 90 €,
horas extra 64 €. Retención de IRPF 9 %. Cotización a la Seguridad Social 6,47 %.
Líquido a percibir: 1.188,32 €.""",
    "es_neg_cuota_gimnasio.txt": """Gimnasio Basic-Fit — Recibo de cuota mensual
Cuota de noviembre: 29,99 €. Acceso a todos los centros. La baja debe solicitarse antes del
día 25 del mes anterior.""",
    "es_neg_campamento.txt": """Campamento de verano Sierra de Gredos — Inscripción
Turno del 1 al 15 de julio para niños de 8 a 14 años. Actividades: senderismo, piragua,
inglés. Precio: 690 €. Traer saco de dormir, linterna y tarjeta sanitaria.""",
    "es_neg_taller_coche.txt": """Taller Mecánico Hermanos Ruiz — Factura de mantenimiento del coche
Cambio de aceite y filtros, revisión de frenos y neumáticos. Matrícula 4471 KJT.
Mano de obra y materiales: 214,60 € con IVA. Próxima revisión a los 15.000 km.""",
    "es_neg_vacuna_gripe.txt": """Campaña de vacunación de la gripe y la COVID-19 para mayores de 60 años
Pida cita en su centro de salud a partir del 2 de octubre. La vacuna es gratuita y se
recomienda especialmente a personas con enfermedades crónicas.""",
    "es_neg_admision_universidad.txt": """Universidad Complutense de Madrid — Admisión
Ha sido admitida en el Grado en Psicología. Plazo de matrícula: del 15 al 22 de julio.
Si no formaliza la matrícula en plazo, perderá la plaza.""",
    # ---------------------------------------------------------------- English
    "en_neg_water_bill.txt": """Thames Water — Your water bill. Period: 1 April to 30 September 2023.
Water supply £98.40, wastewater £112.10. Total £210.50, paid by direct debit.""",
    "en_neg_parking_fine.txt": """Westminster City Council — Penalty Charge Notice
Parking fine for stopping on a single yellow line, Oxford Street, 11:20. £80, reduced to
£40 if paid within 14 days. No points are added to your licence.""",
    "en_neg_car_service.txt": """KwikFit — Car service invoice
Full car service: oil and filter change, brake check, tyre pressure. Total £189.00.""",
    "en_neg_school_trip.txt": """Hillside Primary — School trip letter
Year 4 will visit the Natural History Museum on 14 March. Please return the consent slip
and £12 by Friday. Children need a packed lunch.""",
    "en_neg_home_insurance.txt": """Aviva — Home insurance renewal
Your buildings and contents insurance renews on 2 June. Your new premium is £312. If you
do nothing, your cover will renew automatically.""",
    "en_neg_flu_vaccination.txt": """Oakfield Medical Practice — Flu vaccination clinic
Adults over 65 and people with long-term conditions can book their free flu vaccination
from 1 October.""",
    "en_neg_blood_test.txt": """Riverside Health Centre — Full blood count
Haemoglobin 142 g/L, white cells 6.1, platelets 250. Glucose 5.0 mmol/L. Kidney function
normal. All results normal; no action needed.""",
    "en_neg_deposit_receipt.txt": """Parkside Lettings — Deposit receipt
We confirm receipt of your deposit of £1,096, protected with the Deposit Protection
Service. Your first month's rent is due on move-in day.""",
}

# (query, expected file, language). Meaning only: no content keyword shared with the target.
MEANING = [
    ("gastos médicos", "es_factura_dentista.txt", ES),
    ("multa por ir demasiado rápido", "es_denuncia_radar.txt", ES),
    ("notas escolares de la niña", "es_boletin_evaluacion.txt", ES),
    ("vacunas del bebé", "es_cartilla_inmunizacion.txt", ES),
    ("arreglo de la nevera", "es_reparacion_frigorifico.txt", ES),
    ("cuánto pagamos de calefacción en invierno", "es_factura_gas_enero.txt", ES),
    ("baja del trabajo por dolor de espalda", "es_parte_incapacidad.txt", ES),
    ("alojamiento para las vacaciones de verano", "es_reserva_conil.txt", ES),
    ("herencia del abuelo", "es_aceptacion_adjudicacion.txt", ES),
    ("cambio de compañía de la luz", "es_alta_suministro_electrico.txt", ES),
    ("lo que costó el entierro de la abuela", "es_factura_tanatorio.txt", ES),
    ("problemas de vista del niño", "es_informe_optica.txt", ES),
    ("subida del alquiler", "es_actualizacion_renta.txt", ES),
    ("ayuda para pagar la universidad", "es_resolucion_beca.txt", ES),
    ("nos robaron la bicicleta", "es_denuncia_sustraccion.txt", ES),
    ("humedad en la pared del baño", "es_parte_filtracion.txt", ES),
    ("colesterol alto", "es_analitica_lipidos.txt", ES),
    ("pensión de viudedad", "es_resolucion_supervivencia.txt", ES),
    ("contrato del móvil", "es_alta_linea_portabilidad.txt", ES),
    ("guardería del pequeño", "es_escuela_infantil.txt", ES),
    ("compra de un coche de segunda mano", "es_compraventa_vehiculo.txt", ES),
    ("dieta para adelgazar", "es_plan_nutricional.txt", ES),
    ("instrucciones del lavavajillas", "es_manual_friegaplatos.txt", ES),
    ("seguro del coche", "es_poliza_automovil.txt", ES),
    ("gastos de la comunidad de vecinos", "es_recibo_cuotas_finca.txt", ES),
    ("cita con el especialista del corazón", "es_informe_cardiologia.txt", ES),
    ("medical expenses", "en_dental_invoice.txt", EN),
    ("speeding ticket", "en_camera_notice.txt", EN),
    ("kid's school grades", "en_pupil_report.txt", EN),
    ("broken refrigerator repair", "en_engineer_visit.txt", EN),
    ("high cholesterol", "en_lipid_panel.txt", EN),
    ("rent increase", "en_tenancy_notice.txt", EN),
    ("baby vaccinations", "en_immunisation_record.txt", EN),
    ("car insurance renewal", "en_motor_policy.txt", EN),
    ("funeral costs for grandma", "en_undertaker_invoice.txt", EN),
    ("dishwasher instructions", "en_dishwasher_manual.txt", EN),
]
